use crate::storage::Entry;
use lru::LruCache;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

const IDLE: Duration = Duration::from_secs(5 * 60);
const UNUSED: usize = 0;
const CANDIDATE: usize = 1;
const HOT: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Demand,
    Preload,
    Refresh,
}
struct Weighted {
    entry: Arc<Entry>,
    bytes: u64,
    used: Instant,
}
/// Only browser demand changes recency or promotes entries; disk copies are independent.
pub struct MemoryCache {
    limit: u64,
    bytes: [u64; 3],
    restored: u64,
    closed: bool,
    queues: [LruCache<String, Weighted>; 3],
}
impl MemoryCache {
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            bytes: [0; 3],
            restored: 0,
            closed: false,
            queues: std::array::from_fn(|_| LruCache::unbounded()),
        }
    }
    pub fn bytes(&self) -> u64 {
        self.bytes.iter().sum()
    }
    fn take(&mut self, key: &str) -> Option<(usize, Weighted)> {
        for tier in 0..3 {
            if let Some(value) = self.queues[tier].pop(key) {
                self.bytes[tier] -= value.bytes;
                return Some((tier, value));
            }
        }
        None
    }
    fn insert(&mut self, key: String, value: Weighted, tier: usize) {
        self.bytes[tier] += value.bytes;
        self.queues[tier].put(key, value);
    }
    pub fn get(&mut self, key: &str, demand: bool) -> Option<Arc<Entry>> {
        self.get_at(key, demand, Instant::now())
    }
    fn get_at(&mut self, key: &str, demand: bool, now: Instant) -> Option<Arc<Entry>> {
        let tier = (0..3).find(|&i| self.queues[i].contains(key))?;
        if now.duration_since(self.queues[tier].peek(key)?.used) >= IDLE {
            self.remove(key);
            return None;
        }
        if !demand {
            return self.queues[tier].peek(key).map(|v| v.entry.clone());
        }
        let (_, mut value) = self.take(key)?;
        value.used = now;
        let result = value.entry.clone();
        self.insert(key.into(), value, (tier + 1).min(HOT));
        self.balance();
        Some(result)
    }
    pub fn put(&mut self, key: String, entry: Arc<Entry>, admission: Admission) {
        self.put_at(key, entry, admission, Instant::now());
    }
    fn put_at(&mut self, key: String, entry: Arc<Entry>, admission: Admission, now: Instant) {
        let previous = self
            .take(&key)
            .filter(|(_, v)| now.duration_since(v.used) < IDLE);
        if self.closed || self.limit == 0 {
            return;
        }
        if admission == Admission::Refresh && previous.is_none() {
            return;
        }
        let bytes = Self::entry_bytes(&key, &entry);
        if bytes > self.limit {
            return;
        }
        if bytes > self.limit.saturating_sub(self.bytes()) {
            self.expire_at(now);
        }
        // Background writes may replace their own resident copy, never evict other entries.
        if admission != Admission::Demand && bytes > self.limit.saturating_sub(self.bytes()) {
            return;
        }
        let (tier, used) = if admission == Admission::Demand {
            (
                previous
                    .as_ref()
                    .map_or(CANDIDATE, |(i, _)| (i + 1).min(HOT)),
                now,
            )
        } else {
            previous.map_or((UNUSED, now), |(i, v)| (i, v.used))
        };
        self.insert(key, Weighted { entry, bytes, used }, tier);
        self.balance();
        while self.bytes() > self.limit {
            let tier = (0..3).find(|&i| !self.queues[i].is_empty()).unwrap();
            self.oldest(tier);
        }
    }
    fn oldest(&mut self, tier: usize) -> Option<(String, Weighted)> {
        // ponytail: linear scan only during eviction/demotion; preserves recency across
        // background replacement and hot demotion. Add a time index only if measured costly.
        let key = self.queues[tier]
            .iter()
            .rev()
            .min_by_key(|(_, v)| v.used)?
            .0
            .clone();
        self.take(&key).map(|(_, value)| (key, value))
    }
    fn balance(&mut self) {
        while self.bytes[HOT] > self.limit - self.limit / 5 {
            if let Some((key, value)) = self.oldest(HOT) {
                self.insert(key, value, CANDIDATE);
            } else {
                break;
            }
        }
    }
    pub fn expire_idle(&mut self) {
        self.expire_at(Instant::now());
    }
    fn expire_at(&mut self, now: Instant) {
        for tier in 0..3 {
            let expired: Vec<_> = self.queues[tier]
                .iter()
                .filter(|(_, v)| now.duration_since(v.used) >= IDLE)
                .map(|(k, _)| k.clone())
                .collect();
            for key in expired {
                self.remove(&key);
            }
        }
    }
    fn entry_bytes(key: &str, entry: &Entry) -> u64 {
        512 + 2 * key.encode_utf16().count() as u64
            + entry.body.len() as u64
            + 2 * entry.variant.encode_utf16().count() as u64
            + entry
                .headers
                .iter()
                .map(|(n, v)| 96 + 2 * (n.encode_utf16().count() + v.encode_utf16().count()) as u64)
                .sum::<u64>()
    }
    pub fn resident_keys(&self) -> Vec<String> {
        (0..3)
            .rev()
            .flat_map(|i| {
                let mut entries: Vec<_> = self.queues[i].iter().collect();
                entries.sort_by_key(|(_, v)| std::cmp::Reverse(v.used));
                entries.into_iter().map(|(k, _)| k.clone())
            })
            .collect()
    }
    pub fn restore_remaining(&self) -> u64 {
        if self.closed {
            return 0;
        }
        (self.limit / 2)
            .saturating_sub(self.restored)
            .min((self.limit / 2).saturating_sub(self.bytes()))
    }
    /// Restore at most half the limit, without displacing live requests or adding heat.
    pub fn restore(&mut self, key: String, entry: Arc<Entry>) -> bool {
        if self.closed || self.limit == 0 {
            return false;
        }
        if self.queues.iter().any(|q| q.contains(&key)) {
            return true;
        }
        let bytes = Self::entry_bytes(&key, &entry);
        if bytes > self.restore_remaining() {
            return false;
        }
        self.put(key, entry, Admission::Preload);
        self.restored += bytes;
        true
    }
    pub fn remove(&mut self, key: &str) {
        self.take(key);
    }
    pub fn close(&mut self) {
        self.closed = true;
        for q in &mut self.queues {
            q.clear();
        }
        self.bytes = [0; 3];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry() -> Arc<Entry> {
        Arc::new(Entry {
            checked: 1,
            headers: vec![],
            variant: String::new(),
            body: vec![0; 86],
        })
    }
    // One-letter keys with this entry weigh exactly 600 bytes.
    #[test]
    fn demand_promotes_and_pressure_evicts_unused_then_candidates() {
        let now = Instant::now();
        let mut m = MemoryCache::new(1800);
        m.put_at("a".into(), entry(), Admission::Demand, now);
        m.get_at("a", true, now).unwrap();
        m.put_at("b".into(), entry(), Admission::Demand, now);
        m.put_at("c".into(), entry(), Admission::Preload, now);
        m.put_at("d".into(), entry(), Admission::Preload, now);
        assert!(m.get_at("d", false, now).is_none());
        m.put_at(
            "d".into(),
            entry(),
            Admission::Demand,
            now + Duration::from_secs(1),
        );
        assert!(m.get_at("c", false, now).is_none());
        m.put_at(
            "e".into(),
            entry(),
            Admission::Demand,
            now + Duration::from_secs(2),
        );
        assert!(m.get_at("b", false, now).is_none());
        assert_eq!(m.resident_keys(), vec!["a", "e", "d"]);
        assert_eq!(m.bytes(), 1800);
    }
    #[test]
    fn background_refresh_preserves_idle_time_and_does_not_resurrect() {
        let now = Instant::now();
        let mut m = MemoryCache::new(1800);
        m.put_at("a".into(), entry(), Admission::Demand, now);
        m.put_at("b".into(), entry(), Admission::Preload, now);
        let later = now + Duration::from_secs(299);
        m.put_at("a".into(), entry(), Admission::Refresh, later);
        m.put_at("b".into(), entry(), Admission::Preload, later);
        assert!(m.queues[CANDIDATE].contains("a"));
        assert!(m.queues[UNUSED].contains("b"));
        assert!(m.get_at("a", false, later).is_some());
        m.expire_at(now + IDLE);
        assert_eq!(m.bytes(), 0);
        m.put_at("a".into(), entry(), Admission::Refresh, now + IDLE);
        assert_eq!(m.bytes(), 0);
        m.put_at("c".into(), entry(), Admission::Demand, now);
        m.get_at("c", true, later).unwrap();
        assert!(m.get_at("c", false, now + IDLE).is_some());
        assert!(m.get_at("c", false, later + IDLE).is_none());
    }
    #[test]
    fn demotion_keeps_age_and_idle_entries_leave_before_live_ones() {
        let now = Instant::now();
        let mut m = MemoryCache::new(1800);
        for (index, key) in ["a", "b", "c"].iter().enumerate() {
            let time = now + Duration::from_secs(index as u64);
            m.put_at((*key).into(), entry(), Admission::Demand, time);
            m.get_at(key, true, time).unwrap();
        }
        assert!(m.queues[CANDIDATE].contains("a"));
        assert_eq!(m.queues[CANDIDATE].peek("a").unwrap().used, now);
        m.put_at("d".into(), entry(), Admission::Demand, now + IDLE);
        assert!(m.get_at("a", false, now + IDLE).is_none());
        assert_eq!(m.bytes(), 1800);
        assert!(m.get_at("b", false, now + IDLE).is_some());
        assert!(m.get_at("c", false, now + IDLE).is_some());
    }
    #[test]
    fn restoration_is_capped_at_half_and_never_adds_heat() {
        let mut m = MemoryCache::new(2400);
        assert!(m.restore("a".into(), entry()));
        assert!(m.restore("b".into(), entry()));
        assert!(!m.restore("c".into(), entry()));
        assert_eq!(m.bytes(), 1200);
        assert_eq!(m.queues[UNUSED].len(), 2);
        m.remove("a");
        assert!(!m.restore("c".into(), entry()));
        m.put("d".into(), entry(), Admission::Demand);
        m.put("e".into(), entry(), Admission::Demand);
        assert_eq!(m.bytes(), 1800);
    }
}
