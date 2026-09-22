use gbf_flash_cache_core::{cache::Cache, hash, memory::{Admission, MemoryCache}, network::Network, service::{Fields, Service}, storage::Entry};
use std::{fs, sync::Arc};
fn entry(size: usize) -> Arc<Entry> {
    Arc::new(Entry { checked: 1, headers: vec![], variant: String::new(), body: vec![7; size] })
}
#[test]
fn restore_never_evicts_live_demand() {
    let mut memory = MemoryCache::new(3600);
    memory.put("live".into(), entry(600), Admission::Demand);
    memory.get("live", true).unwrap();
    assert!(!memory.restore("old".into(), entry(600)));
    assert!(memory.get("live", false).is_some());
    assert!(memory.restore("small".into(), entry(20)));
    assert_eq!(memory.resident_keys(), vec!["live", "small"]);
}
#[tokio::test]
async fn residents_roundtrip_without_network_or_counter_hits() {
    let dir = tempfile::tempdir().unwrap();
    let key = hash(b"resource");
    entry(300).write(fs::File::create(dir.path().join(format!("{key}.gfc"))).unwrap()).unwrap();
    fs::write(dir.path().join("memory-resident.json"), serde_json::to_vec(&vec!["../bad".to_owned(), hash(b"missing"), key.clone()]).unwrap()).unwrap();
    let cache = Cache::new(dir.path().into(), 2048, 3600, Network::new(None, &[]).unwrap()).unwrap();
    cache.restore_memory();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while cache.memory_bytes() == 0 { tokio::time::sleep(std::time::Duration::from_millis(10)).await; }
    }).await.unwrap();
    assert_eq!(cache.counters.requests.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(cache.counters.hits.load(std::sync::atomic::Ordering::Relaxed), 0);
    cache.close().await;
    let keys: Vec<String> = serde_json::from_slice(&fs::read(dir.path().join("memory-resident.json")).unwrap()).unwrap();
    assert_eq!(keys, vec![key]);
}
#[tokio::test]
async fn ca_status_regeneration_and_lan_lifecycle() {
    let home = tempfile::tempdir().unwrap();
    let mut service = Service::open(home.path().into()).unwrap();
    assert_eq!(service.command("ca_status", Fields::new()).await.unwrap()["state"], "missing");
    assert!(!home.path().join("ca").exists());
    service.command("ca", Fields::new()).await.unwrap();
    let old = service.command("ca_status", Fields::new()).await.unwrap();
    assert_eq!(old["state"], "valid");
    assert_eq!(old["fingerprint"], service.command("ca_status", Fields::new()).await.unwrap()["fingerprint"]);
    assert!(service.command("ca_regenerate", Fields::new()).await.is_err());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port(); drop(listener);
    let args = Fields::from([("port".into(), port.to_string()), ("lan".into(), "true".into())]);
    service.command("start", args).await.unwrap();
    assert!(service.command("ca_regenerate", Fields::from([("fingerprint".into(),old["fingerprint"].clone())])).await.is_err());
    let socket = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await.unwrap(); drop(socket);
    service.command("stop", Fields::new()).await.unwrap();
    let new = service.command("ca_regenerate", Fields::from([("fingerprint".into(),old["fingerprint"].clone())])).await.unwrap();
    assert_ne!(old["fingerprint"], new["fingerprint"]);
    assert_eq!(new["state"], "valid");
    fs::write(home.path().join("cache/memory-resident.json"), b"[]").unwrap();
    service.command("clear", Fields::new()).await.unwrap();
    assert!(!home.path().join("cache/memory-resident.json").exists());
    service.close().await.unwrap();
}
