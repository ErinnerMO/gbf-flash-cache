use gbf_flash_cache_app::{
    bridge::ffi::*,
    application::{Fields, Service},
};
use std::ffi::{CStr, CString};

#[test]
fn native_host_lifecycle_and_exclusive_data_directory() {
    let home = tempfile::tempdir().unwrap();
    let path = CString::new(home.path().to_str().unwrap()).unwrap();
    unsafe {
        let mut error = std::ptr::null_mut();
        let core = gbf_core_open(path.as_ptr(), &mut error);
        assert!(!core.is_null());
        assert!(error.is_null());
        let duplicate = gbf_core_open(path.as_ptr(), &mut error);
        assert!(duplicate.is_null());
        assert!(!error.is_null());
        gbf_core_free_string(error);
        for command in [r#"{"op":"init"}"#, r#"{"op":"ca"}"#, r#"{"op":"status"}"#] {
            let input = CString::new(command).unwrap();
            let text = gbf_core_command(core, input.as_ptr());
            let response: serde_json::Value =
                serde_json::from_str(CStr::from_ptr(text).to_str().unwrap()).unwrap();
            assert_eq!(response["ok"], true);
            gbf_core_free_string(text);
        }
        let invalid = CString::new(r#"{"op":"unknown"}"#).unwrap();
        let text = gbf_core_command(core, invalid.as_ptr());
        assert!(CStr::from_ptr(text).to_str().unwrap().contains("false"));
        gbf_core_free_string(text);
        gbf_core_free_string(gbf_core_close(core));
        let core = gbf_core_open(path.as_ptr(), &mut error);
        assert!(!core.is_null());
        gbf_core_free_string(gbf_core_close(core));
    }
}
#[tokio::test]
async fn service_settings_export_and_log_privacy() {
    let home = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let mut service = Service::open(home.path().into()).unwrap();
    let mut settings = Fields::new();
    settings.insert("keepLogs".into(), "true".into());
    settings.insert("password".into(), "secret-password".into());
    settings.insert("upstream".into(), "secret-proxy".into());
    service.command("settings", settings).await.unwrap();
    let saved = std::fs::read_to_string(home.path().join("settings.json")).unwrap();
    assert!(!saved.contains("secret"));
    std::fs::create_dir_all(home.path().join("logs")).unwrap();
    let trace =
        gbf_flash_cache_core::trace::Trace::open(&home.path().join("logs/test.jsonl")).unwrap();
    trace.event(
        "REQUEST",
        Some(
            &"https://game.granbluefantasy.jp/raid/12345678?uid=secret-user".parse()
                .unwrap(),
        ),
        serde_json::json!({"method":"GET"}),
    );
    trace.close().await;
    let log = std::fs::read_to_string(home.path().join("logs/test.jsonl")).unwrap();
    assert!(!log.contains("secret-user"));
    assert!(!log.contains("12345678"));
    let destination = output.path().join("logs.zip");
    let args = Fields::from([("path".into(), destination.to_str().unwrap().into())]);
    service.command("export", args).await.unwrap();
    let zip = zip::ZipArchive::new(std::fs::File::open(destination).unwrap()).unwrap();
    assert_eq!(zip.len(), 1);
    assert_eq!(zip.file_names().next().unwrap(), "test.jsonl");
    service.close().await.unwrap();
    assert!(home.path().join("logs/test.jsonl").exists());
    drop(service);
    let mut service = Service::open(home.path().into()).unwrap();
    service
        .command(
            "settings",
            Fields::from([("keepLogs".into(), "false".into())]),
        )
        .await
        .unwrap();
    service.close().await.unwrap();
    assert!(home.path().join("logs/test.jsonl").exists());
    drop(service);
    let _reopened = Service::open(home.path().into()).unwrap();
    assert!(!home.path().join("logs/test.jsonl").exists());
}

#[tokio::test]
async fn custom_directories_migration_and_bind_failure() {
    let home = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let mut service = Service::open(home.path().into()).unwrap();
    std::fs::create_dir_all(home.path().join("cache")).unwrap();
    std::fs::write(home.path().join("cache/test.gfc"), b"cache-test").unwrap();
    let result = service.command("directory", Fields::from([
        ("kind".into(), "cache".into()), ("path".into(), destination.path().to_str().unwrap().into()),
        ("migrate".into(), "true".into()),
    ])).await.unwrap();
    let moved = std::path::PathBuf::from(&result["path"]);
    assert_eq!(std::fs::read(moved.join("test.gfc")).unwrap(), b"cache-test");
    assert!(home.path().join("cache/test.gfc").exists());
    assert_eq!(service.command("status", Fields::new()).await.unwrap()["diskBytes"], "10");
    assert_eq!(service.command("status", Fields::new()).await.unwrap()["diskEntries"], "1");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let manifest = moved.join("memory-resident.json");
    std::fs::write(&manifest, b"[\"previous-resource\"]").unwrap();
    let failure = service.command("start", Fields::from([("port".into(), port.to_string())])).await.unwrap_err();
    assert!(failure.contains("已被占用"));
    assert_eq!(std::fs::read(&manifest).unwrap(), b"[\"previous-resource\"]");
    assert_eq!(service.command("status", Fields::new()).await.unwrap()["running"], "false");
    service.command("clear", Fields::new()).await.unwrap();
    assert!(!moved.join("test.gfc").exists());
    assert_eq!(service.command("status", Fields::new()).await.unwrap()["diskEntries"], "0");
    assert!(home.path().join("cache/test.gfc").exists());
    service.close().await.unwrap();
    drop(service);
    let mut service = Service::open(home.path().into()).unwrap();
    assert_eq!(service.command("init", Fields::new()).await.unwrap()["cachePath"], moved.to_string_lossy());
    let result = service.command("directory", Fields::from([
        ("kind".into(), "logs".into()), ("path".into(), destination.path().to_str().unwrap().into()),
    ])).await.unwrap();
    let logs = std::path::PathBuf::from(&result["path"]);
    std::fs::write(logs.join("test.jsonl"), b"{}\n").unwrap();
    service.close().await.unwrap();
    assert!(logs.join("test.jsonl").exists());
    drop(service);
    let _reopened = Service::open(home.path().into()).unwrap();
    assert!(!logs.join("test.jsonl").exists());
}

#[cfg(windows)]
#[tokio::test]
async fn windows_proxy_password_roundtrip() {
    let home = tempfile::tempdir().unwrap();
    let mut service = Service::open(home.path().into()).unwrap();
    service.command("settings", Fields::from([("password".into(), "test-secret".into())])).await.unwrap();
    service.close().await.unwrap(); drop(service);
    assert!(!std::fs::read_to_string(home.path().join("settings.json")).unwrap().contains("test-secret"));
    let mut service = Service::open(home.path().into()).unwrap();
    assert_eq!(service.command("init", Fields::new()).await.unwrap()["password"], "test-secret");
    service.command("settings", Fields::from([("password".into(), "".into())])).await.unwrap();
    assert!(!service.command("init", Fields::new()).await.unwrap().contains_key("password"));
    service.close().await.unwrap();
}

#[tokio::test]
async fn self_proxy_alias_is_rejected_before_start() {
    let home = tempfile::tempdir().unwrap();
    let mut service = Service::open(home.path().into()).unwrap();
    for host in ["localhost.", "127.0.0.1", "0.0.0.0"] {
        let args = Fields::from([("port".into(), "18765".into()), ("proxy".into(), "true".into()),
            ("host".into(), host.into()), ("proxyPort".into(), "18765".into())]);
        for op in ["start", "probe"] {
            assert!(service.command(op, args.clone()).await.unwrap_err().contains("proxy_loop"));
        }
    }
    gbf_flash_cache_core::tunnel::check_loop("127.0.0.1", 18766, "127.0.0.1:18765".parse().unwrap()).await.unwrap();
    // Binding port zero identifies an actual local interface without sending packets.
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
    if socket.connect("192.0.2.1:9").is_ok() {
        let ip = socket.local_addr().unwrap().ip().to_string();
        assert!(gbf_flash_cache_core::tunnel::check_loop(&ip, 18765, "0.0.0.0:18765".parse().unwrap()).await.is_err());
    }
    service.close().await.unwrap();
}

#[tokio::test]
async fn failed_settings_save_rolls_back_start_and_directory() {
    let home = tempfile::tempdir().unwrap();
    let mut service = Service::open(home.path().into()).unwrap();
    let before = service.command("init", Fields::new()).await.unwrap();
    // Deterministic write failure on both Windows and Unix, without permission changes.
    std::fs::create_dir(home.path().join("settings.json")).unwrap();
    let destination = tempfile::tempdir().unwrap();
    assert!(service.command("directory", Fields::from([
        ("kind".into(), "cache".into()),
        ("path".into(), destination.path().to_string_lossy().into()),
    ])).await.is_err());
    assert_eq!(service.command("init", Fields::new()).await.unwrap()["cachePath"], before["cachePath"]);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    assert!(service.command("start", Fields::from([("port".into(), port.to_string())])).await.is_err());
    assert_eq!(service.command("status", Fields::new()).await.unwrap()["running"], "false");
    let _released = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    service.close().await.unwrap();
}

#[tokio::test]
async fn retired_android_settings_are_not_loaded_or_saved() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("settings.json"),
        r#"{"capture":"true","connection":"local","proxy":"true","host":"127.0.0.1","proxyPort":"7890"}"#).unwrap();
    let mut service = Service::open(home.path().into()).unwrap();
    let saved = service.command("init", Fields::new()).await.unwrap();
    assert!(!saved.contains_key("capture"));
    assert!(!saved.contains_key("connection"));
    assert_eq!(saved["proxy"], "true");
    service.command("settings", Fields::from([("capture".into(), "true".into()), ("connection".into(), "local".into())])).await.unwrap();
    let persisted: Fields = serde_json::from_slice(&std::fs::read(home.path().join("settings.json")).unwrap()).unwrap();
    assert!(!persisted.contains_key("capture"));
    assert!(!persisted.contains_key("connection"));
    assert_eq!(persisted["host"], "127.0.0.1");
    service.close().await.unwrap();
}

#[test]
fn native_handle_survives_unavailable_directories_and_can_repair_them() {
    unsafe fn call(handle: *mut Core, op: &str, args: Fields) -> serde_json::Value {
        let input = CString::new(serde_json::json!({"op":op,"args":args}).to_string()).unwrap();
        unsafe {
            let text = gbf_core_command(handle, input.as_ptr());
            let response = serde_json::from_str(CStr::from_ptr(text).to_str().unwrap()).unwrap();
            gbf_core_free_string(text);
            response
        }
    }
    for kind in ["cache", "logs"] {
        let home = tempfile::tempdir().unwrap();
        let broken = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        // A file where a directory is expected fails consistently on all platforms.
        let path = broken.path().join("unavailable");
        std::fs::write(&path, b"not a directory").unwrap();
        let settings = Fields::from([(format!("{kind}Path"), path.to_string_lossy().into())]);
        std::fs::write(home.path().join("settings.json"), serde_json::to_vec(&settings).unwrap()).unwrap();
        // The unused default must never be opened, even if it too is inaccessible.
        std::fs::write(home.path().join(kind), b"obsolete default").unwrap();
        let home_c = CString::new(home.path().to_str().unwrap()).unwrap();
        unsafe {
            let mut error = std::ptr::null_mut();
            let handle = gbf_core_open(home_c.as_ptr(), &mut error);
            assert!(!handle.is_null());
            assert!(error.is_null());
            let init = call(handle, "init", Fields::new());
            assert_eq!(init["ok"], true);
            assert!(!init["fields"]["directoryWarning"].as_str().unwrap().is_empty());
            assert_eq!(call(handle, "start", Fields::new())["ok"], false);
            assert_eq!(call(handle, "directory", Fields::from([
                ("kind".into(), kind.into()),
                ("path".into(), destination.path().to_string_lossy().into()),
            ]))["ok"], true);
            assert_eq!(call(handle, "status", Fields::new())["fields"]["directoryWarning"], "");
            let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = socket.local_addr().unwrap().port();
            drop(socket);
            assert_eq!(call(handle, "start", Fields::from([("port".into(), port.to_string())]))["ok"], true);
            gbf_core_free_string(gbf_core_close(handle));
            // Reopening uses the repaired custom path, not the broken default.
            let handle = gbf_core_open(home_c.as_ptr(), &mut error);
            assert!(!handle.is_null());
            assert_eq!(call(handle, "init", Fields::new())["fields"]["directoryWarning"], "");
            gbf_core_free_string(gbf_core_close(handle));
        }
    }
}

#[tokio::test]
async fn preference_patch_preserves_unreadable_credentials() {
    let home = tempfile::tempdir().unwrap();
    let original = Fields::from([
        ("protectedPassword".into(), "unreadable-windows-ciphertext".into()),
        ("androidProtectedPassword".into(), "unreadable-android-ciphertext".into()),
    ]);
    std::fs::write(home.path().join("settings.json"), serde_json::to_vec(&original).unwrap()).unwrap();
    let mut app = Service::open(home.path().into()).unwrap();
    app.command("init", Fields::new()).await.unwrap();
    for key in ["keepLogs", "autoRun", "lan", "acceleratorCompatibility"] {
        app.command("settings", Fields::from([(key.into(), "true".into())])).await.unwrap();
    }
    let saved: Fields = serde_json::from_slice(&std::fs::read(home.path().join("settings.json")).unwrap()).unwrap();
    assert_eq!(saved["acceleratorCompatibility"], "true");
    for (key, value) in original { assert_eq!(saved[&key], value); }
    app.close().await.unwrap();
}

#[tokio::test]
async fn partial_proxy_enable_validates_merged_credentials_before_persisting() {
    let home = tempfile::tempdir().unwrap();
    let mut app = Service::open(home.path().into()).unwrap();
    let enable = Fields::from([("proxy".into(), "true".into())]);
    for (protocol, username, valid) in [
        ("HTTP", "bad:name", false), ("HTTPS", "bad:name", false),
        ("SOCKS4", "team:user", true), ("SOCKS5", "user", false),
        ("SOCKS5", "", true), ("HTTP", "user", true),
    ] {
        app.command("settings", Fields::from([
            ("proxy".into(), "false".into()), ("protocol".into(), protocol.into()),
            ("username".into(), username.into()), ("password".into(), "".into()),
        ])).await.unwrap();
        let before = std::fs::read(home.path().join("settings.json")).unwrap();
        assert_eq!(app.command("settings", enable.clone()).await.is_ok(), valid, "{protocol}/{username}");
        if !valid {
            assert_eq!(std::fs::read(home.path().join("settings.json")).unwrap(), before);
            assert_eq!(app.command("init", Fields::new()).await.unwrap()["proxy"], "false");
        }
    }
    app.command("settings", Fields::from([
        ("proxy".into(), "true".into()), ("protocol".into(), "SOCKS4".into()),
        ("username".into(), "team:user".into()),
    ])).await.unwrap();
    assert!(app.command("settings", Fields::from([("protocol".into(), "HTTP".into())])).await.is_err());
    app.close().await.unwrap();
    drop(app);
    // A partial enable cannot treat an unreadable stored secret as an empty password.
    let settings = Fields::from([
        ("proxy".into(), "false".into()), ("protocol".into(), "HTTP".into()),
        ("username".into(), "user".into()), ("protectedPassword".into(), "invalid!".into()),
    ]);
    let original = serde_json::to_vec(&settings).unwrap();
    std::fs::write(home.path().join("settings.json"), &original).unwrap();
    let mut app = Service::open(home.path().into()).unwrap();
    assert!(app.command("settings", enable).await.is_err());
    assert_eq!(std::fs::read(home.path().join("settings.json")).unwrap(), original);
    app.command("settings", Fields::from([("keepLogs".into(), "true".into())])).await.unwrap();
    app.close().await.unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn partial_enable_decrypts_saved_socks5_password_without_rewriting_it() {
    let home = tempfile::tempdir().unwrap();
    let mut app = Service::open(home.path().into()).unwrap();
    app.command("settings", Fields::from([
        ("proxy".into(), "false".into()), ("protocol".into(), "SOCKS5".into()),
        ("username".into(), "team:user".into()), ("password".into(), "secret".into()),
    ])).await.unwrap();
    let before: Fields = serde_json::from_slice(&std::fs::read(home.path().join("settings.json")).unwrap()).unwrap();
    app.command("settings", Fields::from([("proxy".into(), "true".into())])).await.unwrap();
    let after: Fields = serde_json::from_slice(&std::fs::read(home.path().join("settings.json")).unwrap()).unwrap();
    assert_eq!(before["protectedPassword"], after["protectedPassword"]);
    assert_eq!(after["proxy"], "true");
    app.close().await.unwrap();
}

#[tokio::test]
async fn deferred_startup_log_cleanup_runs_once_after_lock_recovery() {
    for keep in [false, true] {
        for recovery in ["status", "start", "export"] {
            let home = tempfile::tempdir().unwrap();
            let output = tempfile::tempdir().unwrap();
            let logs = home.path().join("logs");
            std::fs::create_dir(&logs).unwrap();
            let lock = std::fs::File::create(logs.join(".gbf-flash-cache.lock")).unwrap();
            lock.try_lock().unwrap();
            let old = logs.join("old.jsonl");
            std::fs::write(&old, b"{}\n").unwrap();
            std::fs::write(logs.join("unrelated.txt"), b"keep").unwrap();
            std::fs::write(home.path().join("settings.json"),
                format!(r#"{{"keepLogs":"{keep}"}}"#)).unwrap();
            let mut service = Service::open(home.path().into()).unwrap();
            assert!(!service.command("status", Fields::new()).await.unwrap()["directoryWarning"].is_empty());
            assert!(old.exists()); // Never touch the other owner's logs.
            drop(lock);
            let destination = output.path().join("logs.zip");
            let export = Fields::from([("path".into(), destination.to_str().unwrap().into())]);
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            drop(listener);
            let args = match recovery {
                "export" => export.clone(),
                "start" => Fields::from([("port".into(), port.to_string())]),
                _ => Fields::new(),
            };
            service.command(recovery, args).await.unwrap();
            assert_eq!(old.exists(), keep, "{recovery}");
            service.command("stop", Fields::new()).await.unwrap();
            let current = logs.join("current.jsonl");
            std::fs::write(&current, b"{}\n").unwrap();
            assert_eq!(service.command("status", Fields::new()).await.unwrap()["directoryWarning"], "");
            service.command("export", export).await.unwrap();
            let mut archive = zip::ZipArchive::new(std::fs::File::open(destination).unwrap()).unwrap();
            assert_eq!(archive.by_name("old.jsonl").is_ok(), keep);
            assert!(archive.by_name("current.jsonl").is_ok());
            assert!(logs.join("unrelated.txt").exists());
            service.close().await.unwrap();
        }
    }
}

#[tokio::test]
async fn theme_preference_survives_reopen_and_partial_settings_updates() {
    let home = tempfile::tempdir().unwrap();
    let mut service = Service::open(home.path().into()).unwrap();
    service.command("settings", Fields::from([("themeMode".into(), "light".into())]))
        .await.unwrap();
    service.command("settings", Fields::from([("keepLogs".into(), "true".into())]))
        .await.unwrap();
    service.close().await.unwrap();
    drop(service);
    let mut service = Service::open(home.path().into()).unwrap();
    let settings = service.command("init", Fields::new()).await.unwrap();
    assert_eq!(settings["themeMode"], "light");
    assert_eq!(settings["keepLogs"], "true");
    service.close().await.unwrap();
}
