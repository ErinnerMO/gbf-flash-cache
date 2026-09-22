use crate::refs::References;
use std::collections::{BTreeSet, HashMap, HashSet};
use url::Url;

pub const CAPACITY: usize = 65536;
#[derive(Default)]
struct Group {
    last: f64,
    served: u64,
    sequence: u64,
    order: HashMap<String, u64>,
    failures: u32,
    blocked: bool,
}
#[derive(Clone, Debug)]
pub struct Job {
    pub url: String,
    pub sequence: u64,
    pub sources: BTreeSet<String>,
    pub speculative: bool,
    pub kind: u8,
}
#[derive(Default)]
pub struct Scheduler {
    sequence: u64,
    turn: u64,
    scene_until: f64,
    groups: HashMap<String, Group>,
    dependencies: HashMap<String, BTreeSet<String>>,
    weak: HashMap<String, BTreeSet<String>>,
    recent: HashMap<String, f64>,
    known: HashMap<String, Job>,
    queue: Vec<String>,
    ready: HashSet<String>,
    demand_running: HashSet<String>,
    window_sources: HashMap<String, f64>,
    pub limit: Option<&'static str>,
}
fn path(url: &str) -> String {
    Url::parse(url)
        .map(|u| u.path().to_owned())
        .unwrap_or_default()
}
pub fn resource_type(url: &str) -> u8 {
    let p = path(url);
    if p.ends_with(".js") {
        0
    } else if p.ends_with(".css") {
        1
    } else {
        2
    }
}
fn heat(g: &Group, at: f64) -> u8 {
    if at - g.last < 2. {
        0
    } else if at - g.last < 10. {
        1
    } else {
        2
    }
}
impl Scheduler {
    pub fn demand(&mut self, url: &str, at: f64) {
        if !at.is_finite() || Url::parse(url).is_err() {
            return;
        }
        if self.recent.len() >= CAPACITY && !self.recent.contains_key(url) {
            self.limit = Some("recent");
            return;
        }
        self.recent.insert(url.into(), at);
        self.demand_running.insert(url.into());
        let u = Url::parse(url).unwrap();
        {
            if matches!(
                u.host_str(),
                Some("game.granbluefantasy.jp" | "gbf.game.mbga.jp")
            ) && [
                "/quest/quest_data",
                "/quest/raid_deck_data_create",
                "/quest/coopraid_deck_data_create",
            ]
            .contains(&u.path())
            {
                self.scene_until = at + 30.;
            }
            if at < self.scene_until {
                self.window_sources.insert(url.into(), self.scene_until);
            }
        }
        if let Some(j) = self.known.get(url) {
            for s in &j.sources {
                let g = self.groups.get_mut(s).unwrap();
                g.last = g.last.max(at);
            }
        }
    }
    pub fn response(&mut self, url: &str, successful_asset: bool) {
        self.demand_running.remove(url);
        if successful_asset {
            self.ready.insert(url.into());
        }
    }
    fn reach(&self) -> HashMap<String, f64> {
        let mut roots: Vec<_> = self.recent.iter().collect();
        roots.sort_by(|a, b| b.1.total_cmp(a.1));
        let mut reached = HashMap::new();
        for (root, at) in roots {
            let mut pending = vec![root.as_str()];
            while let Some(u) = pending.pop() {
                if reached.contains_key(u) {
                    continue;
                }
                reached.insert(u.to_owned(), *at);
                if let Some(edges) = self.dependencies.get(u) {
                    pending.extend(edges.iter().map(String::as_str));
                }
            }
        }
        reached
    }
    pub fn discover(&mut self, source: &str, refs: &References, at: f64) {
        if !at.is_finite() || refs.is_empty() || self.groups.get(source).is_some_and(|g| g.blocked)
        {
            return;
        }
        if self.groups.len() >= CAPACITY && !self.groups.contains_key(source) {
            self.limit = Some("groups");
            return;
        }
        let chain = self.reach();
        let group = self.groups.entry(source.into()).or_insert_with(|| Group {
            last: -1e9,
            ..Group::default()
        });
        group.last = group.last.max(*chain.get(source).unwrap_or(&-1e9));
        let inherited =
            self.known.get(source).is_some_and(|j| j.speculative) && !chain.contains_key(source);
        let mut targets: Vec<_> = refs.iter().collect();
        targets.sort_by_key(|(u, _)| resource_type(u));
        for (target, deferred) in targets {
            if !self.known.contains_key(target) {
                if self.known.len() >= CAPACITY || self.queue.len() >= CAPACITY {
                    self.limit = Some("candidates");
                    break;
                }
                if Url::parse(target).is_err() {
                    continue;
                }
                self.sequence += 1;
                self.known.insert(
                    target.clone(),
                    Job {
                        url: target.clone(),
                        sequence: self.sequence,
                        sources: BTreeSet::new(),
                        speculative: inherited || *deferred,
                        kind: resource_type(target),
                    },
                );
                self.queue.push(target.clone());
            }
            let j = self.known.get_mut(target).unwrap();
            j.speculative &= inherited || *deferred;
            if j.sources.insert(source.into()) {
                group.sequence += 1;
                group.order.insert(target.clone(), group.sequence);
            }
            if !deferred {
                self.dependencies
                    .entry(source.into())
                    .or_default()
                    .insert(target.clone());
            } else {
                self.weak
                    .entry(source.into())
                    .or_default()
                    .insert(target.clone());
            }
        }
    }
    fn window_reach(&self, weak: bool, at: f64) -> HashSet<String> {
        let mut reached = HashSet::new();
        let mut pending: Vec<_> = self
            .window_sources
            .iter()
            .filter(|(_, until)| at < **until)
            .map(|(url, _)| url.as_str())
            .collect();
        while let Some(u) = pending.pop() {
            if !reached.insert(u.to_owned()) {
                continue;
            }
            if let Some(edges) = self.dependencies.get(u) {
                pending.extend(edges.iter().map(String::as_str));
            }
            if weak {
                if let Some(edges) = self.weak.get(u) {
                    pending.extend(edges.iter().map(String::as_str));
                }
            }
        }
        reached
    }
    pub fn take(&mut self, at: f64) -> Option<Job> {
        if !at.is_finite() {
            return None;
        }
        self.queue
            .retain(|u| !self.ready.contains(u) && !self.demand_running.contains(u));
        if self.queue.is_empty() {
            return None;
        }
        let chain = self.reach();
        for (u, t) in &chain {
            if let Some(g) = self.groups.get_mut(u) {
                g.last = g.last.max(*t);
            }
        }
        self.window_sources.retain(|_, until| at < *until);
        let strong = self.window_reach(false, at);
        let all = self.window_reach(true, at);
        let mut best: Option<(usize, String, Vec<f64>)> = None;
        // ponytail: score each source edge once per available slot; index after measured dispatch cost warrants it.
        for (i, u) in self.queue.iter().enumerate() {
            let j = &self.known[u];
            for s in j.sources.iter().filter(|s| !self.groups[*s].blocked) {
                let g = &self.groups[s];
                let kind = j.kind as f64;
                let served = g.served as f64;
                let seq = j.sequence as f64;
                let rank = {
                    let edge = self.dependencies.get(s).is_some_and(|e| e.contains(u));
                    let tier = if strong.contains(s) && edge {
                        0
                    } else if all.contains(s) {
                        1
                    } else {
                        2 + j.kind
                    };
                    if tier < 2 {
                        vec![tier as f64, kind, served, g.order[u] as f64, seq]
                    } else {
                        vec![
                            tier as f64,
                            if edge && (!j.speculative || chain.contains_key(s)) {
                                0.
                            } else {
                                1.
                            },
                            heat(g, at) as f64,
                            served,
                            g.order[u] as f64,
                            seq,
                        ]
                    }
                };
                if best.as_ref().is_none_or(|(_, _, r)| rank < *r) {
                    best = Some((i, s.clone(), rank));
                }
            }
        }
        let (index, source, _) = best?;
        let url = self.queue.remove(index);
        self.turn += 1;
        self.groups.get_mut(&source).unwrap().served = self.turn;
        self.known.get(&url).cloned()
    }
    pub fn completion(&mut self, url: &str, code: u16) {
        if code == 200 {
            self.ready.insert(url.into());
        } else if matches!(code, 404 | 410) {
            if let Some(j) = self.known.get(url) {
                for s in &j.sources {
                    let g = self.groups.get_mut(s).unwrap();
                    g.failures += 1;
                    g.blocked = g.failures >= 5;
                }
            }
            self.queue.retain(|u| {
                self.known[u]
                    .sources
                    .iter()
                    .any(|s| !self.groups[s].blocked)
            });
        }
    }
    pub fn pending(&self) -> usize {
        self.queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const A: &str = "https://game.granbluefantasy.jp/quest/quest_data?round=a";
    const B: &str = "https://game.granbluefantasy.jp/quest/raid_deck_data_create?round=b";
    const IMAGE: &str = "https://cdn.example/assets/a.png";
    const JS: &str = "https://cdn.example/assets/b.js";

    #[test]
    fn overlapping_windows_keep_old_priority_but_not_extend_old_deadline() {
        for weak in [false, true] {
            for (at, expected) in [(11., IMAGE), (30., JS)] {
                let mut s = Scheduler::default();
                s.demand(A, 0.);
                s.discover(A, &References::from([(IMAGE.into(), weak)]), 1.);
                s.demand(B, 10.);
                // An unrelated general-scene JS competes with the old battle image.
                s.discover(
                    "https://game.granbluefantasy.jp/other",
                    &References::from([(JS.into(), false)]),
                    11.,
                );
                assert_eq!(s.take(at).unwrap().url, expected);
                assert!(s.window_reach(false, 30.).contains(B));
                assert!(!s.window_reach(false, 30.).contains(A));
                assert!(s.window_reach(true, 40.).is_empty());
            }
        }
    }

    #[test]
    fn new_source_can_extend_shared_resource_without_extending_others() {
        let mut s = Scheduler::default();
        s.demand(A, 0.);
        s.discover(
            A,
            &References::from([(IMAGE.into(), false), (JS.into(), false)]),
            1.,
        );
        s.demand(B, 10.);
        s.discover(B, &References::from([(IMAGE.into(), false)]), 11.);
        let reached = s.window_reach(false, 30.);
        assert!(reached.contains(IMAGE));
        assert!(!reached.contains(JS));
        assert!(s.window_reach(false, 40.).is_empty());
    }
}
