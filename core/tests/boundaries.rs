use gbf_flash_cache_core::service::{Fields, Service};

#[tokio::test]
async fn engine_ignores_application_settings_and_preserves_logs() {
    let home = tempfile::tempdir().unwrap();
    let logs = home.path().join("logs");
    std::fs::create_dir(&logs).unwrap();
    std::fs::write(logs.join("existing.jsonl"), b"preserve").unwrap();
    std::fs::write(home.path().join("settings.json"), b"invalid app settings").unwrap();
    let mut service = Service::open(home.path().into()).unwrap();
    let paths = tempfile::tempdir().unwrap();
    let cache = paths.path().join("cache");
    let trace = paths.path().join("trace");
    service
        .configure(cache.clone(), trace.clone(), vec![])
        .unwrap();
    let init = service.command("init", Fields::new()).await.unwrap();
    assert_eq!(init["cachePath"], cache.canonicalize().unwrap().to_string_lossy());
    assert_eq!(init["logsPath"], trace.canonicalize().unwrap().to_string_lossy());
    for op in [
        "settings",
        "startup",
        "ca_install",
        "ca_uninstall",
        "export",
    ] {
        assert!(service.command(op, Fields::new()).await.is_err(), "{op}");
    }
    service.close().await.unwrap();
    assert_eq!(
        std::fs::read(logs.join("existing.jsonl")).unwrap(),
        b"preserve"
    );
    assert_eq!(
        std::fs::read(home.path().join("settings.json")).unwrap(),
        b"invalid app settings"
    );
}

#[tokio::test]
async fn resolved_upstream_still_rejects_self_loop() {
    let home = tempfile::tempdir().unwrap();
    let mut service = Service::open(home.path().into()).unwrap();
    let args = Fields::from([("port".into(), "18765".into()),
        ("upstream".into(), "http://127.0.0.1:18765".into())]);
    for op in ["start", "probe"] {
        assert!(service.command(op, args.clone()).await.unwrap_err().contains("proxy_loop"));
    }
    service.close().await.unwrap();
}
