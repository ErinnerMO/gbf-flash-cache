use gbf_flash_cache_app::application::{Fields, Service};
use gbf_flash_cache_core::{
    cache::Cache, certificates::Authority, engine::Engine, gateway::Gateway, network::Network,
    tunnel::Route,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use url::Url;
#[tokio::test]
async fn proxy_credentials_socks4_and_outer_tls_regressions() {
    let home = tempfile::tempdir().unwrap();
    let ca = Authority::open(&home.path().join("ca")).unwrap();
    let cache = Cache::new(
        home.path().join("cache"),
        0,
        3600,
        Network::new(None, &[]).unwrap(),
    )
    .unwrap();
    let engine = Engine::new(cache, Url::parse(gbf_flash_cache_core::CDN).unwrap()).unwrap();
    let gateway =
        Gateway::start_with_lan(0, engine, &ca, &[], Route::new(None, &[]).unwrap(), true)
            .await
            .unwrap();
    let route = Route::new(None, &[ca.certificate.clone()]).unwrap();
    let stream = TcpStream::connect(("127.0.0.1", gateway.address.port()))
        .await
        .unwrap();
    assert!(route.secure(stream, "127.0.0.1").await.is_ok());
    let stream = TcpStream::connect(("127.0.0.2", gateway.address.port()))
        .await
        .unwrap();
    assert!(route.secure(stream, "127.0.0.2").await.is_ok());
    let stream = TcpStream::connect(("127.0.0.1", gateway.address.port()))
        .await
        .unwrap();
    assert_eq!(
        route.secure(stream, "192.168.1.23").await.err().unwrap(),
        "tls_certificate"
    );
    // Verify the actual LAN IP too when this test host has a routed interface.
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
    if socket.connect("192.0.2.1:9").is_ok() {
        let ip = socket.local_addr().unwrap().ip();
        let stream = TcpStream::connect((ip, gateway.address.port()))
            .await
            .unwrap();
        assert!(route.secure(stream, &ip.to_string()).await.is_ok());
    }
    gateway.close().await;
    let app_home = tempfile::tempdir().unwrap();
    let mut app = Service::open(app_home.path().into()).unwrap();
    let free = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let mut args = Fields::from([
        ("port".into(), port.to_string()),
        ("proxy".into(), "true".into()),
        ("protocol".into(), "SOCKS4".into()),
        ("host".into(), "127.0.0.1".into()),
        ("proxyPort".into(), "1080".into()),
        ("username".into(), "".into()),
        ("password".into(), "old-secret".into()),
    ]);
    app.command("start", args.clone()).await.unwrap();
    app.command("stop", Fields::new()).await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    args.insert("protocol".into(), "HTTP".into());
    args.insert(
        "proxyPort".into(),
        listener.local_addr().unwrap().port().to_string(),
    );
    args.insert("username".into(), "u%41".into());
    args.insert("password".into(), "p%42".into());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut b = [0];
            stream.read_exact(&mut b).await.unwrap();
            request.push(b[0]);
            if request.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        stream
            .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
        String::from_utf8(request).unwrap()
    });
    assert!(app.command("probe", args).await.is_err());
    let request = server.await.unwrap();
    let auth = request
        .lines()
        .find(|s| s.to_lowercase().starts_with("proxy-authorization"))
        .unwrap();
    assert!(auth.ends_with("dSU0MTpwJTQy"), "{auth}");
    app.close().await.unwrap();
}

#[tokio::test]
async fn unavailable_saved_password_does_not_block_init() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("settings.json"),
        r#"{"protectedPassword":"not-valid-base64!","proxy":"true","port":"8765"}"#,
    )
    .unwrap();
    let mut app = Service::open(home.path().into()).unwrap();
    let result = app.command("init", Fields::new()).await.unwrap();
    assert_eq!(result["password"], "");
    assert!(result.contains_key("passwordWarning"));
    assert!(!result.contains_key("protectedPassword"));
    assert_eq!(result["proxy"], "true");
    app.close().await.unwrap();
}

#[tokio::test]
async fn host_roots_control_https_upstream_trust() {
    use std::sync::Arc;
    use tokio_rustls::{rustls, TlsAcceptor};
    let home = tempfile::tempdir().unwrap();
    let ca = Authority::open(&home.path().join("ca")).unwrap();
    let other = Authority::open(&home.path().join("other")).unwrap();
    assert!(Service::open_with_roots(home.path().join("empty"), vec![]).is_err());
    for (trusted, host) in [
        (true, "127.0.0.1"),
        (false, "127.0.0.1"),
        (true, "::1"),
        (false, "::1"),
    ] {
        let leaf = ca.leaf(host).unwrap();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.certificate.into()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(leaf.key).into(),
        )
        .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind((host, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let Ok(mut stream) = acceptor.accept(socket).await else {
                return false;
            };
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                if stream.read_exact(&mut byte).await.is_err() {
                    return false;
                }
                request.push(byte[0]);
            }
            assert!(request.starts_with(b"CONNECT game.granbluefantasy.jp:443 "));
            stream
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
            true
        });
        let root = if trusted { &ca } else { &other };
        let mut app = Service::open_with_roots(
            home.path().join(format!("app-{trusted}-{}", host.len())),
            vec![root.certificate.clone()],
        )
        .unwrap();
        let args = Fields::from([
            ("proxy".into(), "true".into()),
            ("protocol".into(), "HTTPS".into()),
            ("host".into(), host.into()),
            ("proxyPort".into(), port.to_string()),
        ]);
        assert!(app.command("probe", args).await.is_err()); // Mock rejects CONNECT; no game request is sent.
        assert_eq!(server.await.unwrap(), trusted);
        app.close().await.unwrap();
    }
}

#[tokio::test]
async fn ipv6_http_relay_and_upstream() {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let origin = TcpListener::bind("[::1]:0").await.unwrap();
        let port = origin.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            for proxy in [false, true] {
                let (mut stream, _) = origin.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0]; stream.read_exact(&mut byte).await.unwrap(); request.push(byte[0]);
                }
                let request = String::from_utf8(request).unwrap();
                if proxy {
                    assert!(request.starts_with("CONNECT [::1]:80 HTTP/1.1"));
                    stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
                } else {
                    assert!(request.starts_with("GET /probe HTTP/1.1"));
                    assert!(request.to_ascii_lowercase().contains(&format!("host: [::1]:{port}")));
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").await.unwrap();
                }
            }
        });
        let home = tempfile::tempdir().unwrap();
        let ca = Authority::open(&home.path().join("ca")).unwrap();
        let cache = Cache::new(home.path().join("cache"), 0, 3600, Network::new(None, &[]).unwrap()).unwrap();
        let engine = Engine::new(cache, Url::parse(gbf_flash_cache_core::CDN).unwrap()).unwrap();
        let gateway = Gateway::start(0, engine, &ca, &[], Route::new(None, &[]).unwrap()).await.unwrap();
        let mut browser = TcpStream::connect(gateway.address).await.unwrap();
        browser.write_all(format!("GET http://[::1]:{port}/probe HTTP/1.1\r\nHost: [::1]:{port}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        let mut reply = Vec::new(); browser.read_to_end(&mut reply).await.unwrap();
        assert!(reply.starts_with(b"HTTP/1.1 200"), "{}", String::from_utf8_lossy(&reply));
        let route = Route::new(Some(&format!("http://[::1]:{port}")), &[]).unwrap();
        route.open("[::1]", 80).await.unwrap();
        server.await.unwrap(); gateway.close().await;
    }).await.unwrap();
}

#[tokio::test]
async fn socks_colon_credentials_reach_proxy() {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        for protocol in ["socks4", "socks5"] {
            let listener = TcpListener::bind("[::1]:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                if protocol == "socks4" {
                    let mut header = [0; 8];
                    stream.read_exact(&mut header).await.unwrap();
                    assert_eq!(header[0], 4);
                    let mut user = Vec::new();
                    loop {
                        let byte = stream.read_u8().await.unwrap();
                        if byte == 0 {
                            break;
                        }
                        user.push(byte);
                    }
                    assert_eq!(user, b"team:user");
                    stream
                        .write_all(&[0, 90, 0, 80, 127, 0, 0, 1])
                        .await
                        .unwrap();
                } else {
                    assert_eq!(stream.read_u8().await.unwrap(), 5);
                    let count = stream.read_u8().await.unwrap();
                    let mut methods = vec![0; count as usize];
                    stream.read_exact(&mut methods).await.unwrap();
                    assert!(methods.contains(&2));
                    stream.write_all(&[5, 2]).await.unwrap();
                    assert_eq!(stream.read_u8().await.unwrap(), 1);
                    let length = stream.read_u8().await.unwrap();
                    let mut user = vec![0; length as usize];
                    stream.read_exact(&mut user).await.unwrap();
                    assert_eq!(user, b"team:user");
                    let length = stream.read_u8().await.unwrap();
                    let mut password = vec![0; length as usize];
                    stream.read_exact(&mut password).await.unwrap();
                    assert_eq!(password, b"secret");
                    stream.write_all(&[1, 0]).await.unwrap();
                    let mut request = [0; 10];
                    stream.read_exact(&mut request).await.unwrap();
                    assert_eq!(&request[..4], &[5, 1, 0, 1]);
                    stream
                        .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 80])
                        .await
                        .unwrap();
                }
            });
            let credentials = if protocol == "socks4" {
                "team%3Auser"
            } else {
                "team%3Auser:secret"
            };
            let route = Route::new(
                Some(&format!("{protocol}://{credentials}@[::1]:{port}")),
                &[],
            )
            .unwrap();
            route.open("127.0.0.1", 80).await.unwrap();
            server.await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn credentials_are_checked_before_save_and_start() {
    let home = tempfile::tempdir().unwrap();
    let mut app = Service::open(home.path().into()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let long = "x".repeat(256);
    let maximum = "x".repeat(255);
    for (protocol, user, password, valid) in [
        ("HTTP", "team:user", "secret", false),
        ("SOCKS4", "team:user", "ignored", true),
        ("SOCKS5", "team:user", "secret", true),
        ("SOCKS5", "team:user", "", false),
        ("SOCKS5", "", "secret", false),
        ("SOCKS5", "", "", true),
        ("SOCKS4", "bad\0user", "", false),
        ("SOCKS4", long.as_str(), "", false),
        ("SOCKS4", maximum.as_str(), "", true),
        ("SOCKS5", "user", long.as_str(), false),
        ("SOCKS5", maximum.as_str(), maximum.as_str(), true),
        ("HTTP", "user", "", true),
    ] {
        let args = Fields::from([
            ("port".into(), port.to_string()),
            ("proxy".into(), "true".into()),
            ("host".into(), "[::1]".into()),
            ("proxyPort".into(), "1080".into()),
            ("protocol".into(), protocol.into()),
            ("username".into(), user.into()),
            ("password".into(), password.into()),
        ]);
        assert_eq!(
            app.command("settings", args.clone()).await.is_ok(),
            valid,
            "save {protocol}/{user}"
        );
        assert_eq!(
            app.command("start", args.clone()).await.is_ok(),
            valid,
            "start {protocol}/{user}"
        );
        app.command("stop", Fields::new()).await.unwrap();
        if !valid {
            assert!(app.command("probe", args).await.is_err());
        }
    }
    app.close().await.unwrap();
}

#[tokio::test]
async fn socks4_combined_encoding_limit_returns_error_without_panic() {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let user = "u".repeat(255);
        let route = Route::new(
            Some(&format!(
                "socks4://{user}@{}",
                listener.local_addr().unwrap()
            )),
            &[],
        )
        .unwrap();
        let domain = format!(
            "{}.{}.{}.{}",
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(61)
        );
        assert_eq!(domain.len(), 253);
        assert!(route
            .open(&domain, 80)
            .await
            .err()
            .unwrap()
            .contains("proxy_request_too_long"));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), listener.accept())
                .await
                .is_err()
        );
        // Exactly 513 bytes remains usable, rather than banning all long usernames.
        let domain = domain[..248].to_string();
        let expected = domain.clone();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut encoded = [0; 513];
            stream.read_exact(&mut encoded).await.unwrap();
            assert_eq!(&encoded[264..512], expected.as_bytes());
            assert_eq!(encoded[512], 0);
            stream.write_all(&[0, 90, 0, 0, 0, 0, 0, 0]).await.unwrap();
        });
        route.open(&domain, 80).await.unwrap();
        server.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn socks_domain_case_preserves_inspection() {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let home = tempfile::tempdir().unwrap();
        let ca = Authority::open(&home.path().join("ca")).unwrap();
        let cache = Cache::new(
            home.path().join("cache"),
            0,
            3600,
            Network::new(None, &[]).unwrap(),
        )
        .unwrap();
        let origin = Url::parse("https://case.invalid/").unwrap();
        let engine = Engine::new(cache, origin.clone()).unwrap();
        let gateway = Gateway::start(0, engine, &ca, &[origin], Route::new(None, &[]).unwrap())
            .await
            .unwrap();
        let tls = Route::new(None, &[ca.certificate.clone()]).unwrap();
        for protocol in ["socks4", "socks5"] {
            for host in ["case.invalid", "CASE.INVALID", "CaSe.InVaLiD"] {
                let route =
                    Route::new(Some(&format!("{protocol}://{}", gateway.address)), &[]).unwrap();
                let stream = route.open(host, 443).await.unwrap();
                // This CA-signed handshake only exists on the inspected branch; no DNS/server required.
                tls.secure(stream, "case.invalid").await.unwrap();
            }
        }
        gateway.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ipv6_upstream_can_share_ipv4_listener_port() {
    let upstream = TcpListener::bind("[::1]:0").await.unwrap();
    let port = upstream.local_addr().unwrap().port();
    let home = tempfile::tempdir().unwrap();
    let mut app = Service::open(home.path().into()).unwrap();
    for lan in ["false", "true"] {
        let args = Fields::from([
            ("port".into(), port.to_string()),
            ("lan".into(), lan.into()),
            ("proxy".into(), "true".into()),
            ("host".into(), "::1".into()),
            ("proxyPort".into(), port.to_string()),
        ]);
        app.command("start", args).await.unwrap();
        assert_eq!(
            app.command("status", Fields::new()).await.unwrap()["running"],
            "true"
        );
        app.command("stop", Fields::new()).await.unwrap();
    }
    let listen = format!("127.0.0.1:{port}").parse().unwrap();
    for host in ["127.0.0.1", "::ffff:127.0.0.1"] {
        assert!(gbf_flash_cache_core::tunnel::check_loop(host, port, listen)
            .await
            .is_err());
    }
    gbf_flash_cache_core::tunnel::check_loop("127.0.0.2", port, listen)
        .await
        .unwrap();
    app.close().await.unwrap();
}

#[tokio::test]
async fn failed_migration_can_retry_same_directory_and_preserves_existing_files() {
    let home = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let mut app = Service::open(home.path().into()).unwrap();
    let old = home.path().join("cache/item.gfc");
    std::fs::write(&old, b"original").unwrap();
    std::fs::create_dir(home.path().join("settings.json")).unwrap();
    let args = Fields::from([
        ("kind".into(), "cache".into()),
        ("migrate".into(), "true".into()),
        ("path".into(), dest.path().to_string_lossy().into()),
    ]);
    assert!(app
        .command("directory", args.clone())
        .await
        .unwrap_err()
        .contains("无法保存设置"));
    let target = dest.path().join("gbf-flash-cache-cache");
    assert!(!target.join("item.gfc").exists());
    assert_eq!(std::fs::read(&old).unwrap(), b"original");
    assert_eq!(
        app.command("init", Fields::new()).await.unwrap()["cachePath"],
        home.path().join("cache").canonicalize().unwrap().to_string_lossy()
    );
    std::fs::remove_dir(home.path().join("settings.json")).unwrap();
    // An existing destination must not be deleted by a failed migration either.
    std::fs::write(target.join("item.gfc"), b"existing").unwrap();
    assert!(app.command("directory", args.clone()).await.is_err());
    assert_eq!(std::fs::read(target.join("item.gfc")).unwrap(), b"existing");
    std::fs::remove_file(target.join("item.gfc")).unwrap();
    app.command("directory", args).await.unwrap();
    assert_eq!(std::fs::read(target.join("item.gfc")).unwrap(), b"original");
    app.close().await.unwrap();
}

#[tokio::test]
async fn custom_directories_are_exclusive_even_while_stopped() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let mut first = Service::open(a.path().into()).unwrap();
    let mut second = Service::open(b.path().into()).unwrap();
    for kind in ["cache", "logs"] {
        let args = Fields::from([
            ("kind".into(), kind.into()),
            ("path".into(), dest.path().to_string_lossy().into()),
        ]);
        first.command("directory", args.clone()).await.unwrap();
        assert!(second
            .command("directory", args)
            .await
            .unwrap_err()
            .contains("另一个实例"));
    }
    let available = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = available.local_addr().unwrap().port();
    drop(available);
    first
        .command("start", Fields::from([("port".into(), port.to_string())]))
        .await
        .unwrap();
    let cache = dest.path().join("gbf-flash-cache-cache/item.gfc");
    let log = dest.path().join("gbf-flash-cache-logs/active.jsonl");
    std::fs::write(&cache, b"cache").unwrap();
    std::fs::write(&log, b"log").unwrap();
    second.command("clear", Fields::new()).await.unwrap();
    assert!(cache.exists());
    drop(second);
    // Saved custom paths on a different installation must fail before startup log cleanup.
    for kind in ["cache", "logs"] {
        let settings = Fields::from([(
            format!("{kind}Path"),
            dest.path()
                .join(format!("gbf-flash-cache-{kind}"))
                .to_string_lossy()
                .into(),
        )]);
        std::fs::write(
            b.path().join("settings.json"),
            serde_json::to_vec(&settings).unwrap(),
        )
        .unwrap();
        let mut blocked = Service::open(b.path().into()).unwrap();
        assert!(blocked.command("init", Fields::new()).await.unwrap()["directoryWarning"].contains("另一个实例"));
        if kind == "cache" {
            assert!(blocked.command("clear", Fields::new()).await.is_err());
            let target = tempfile::tempdir().unwrap();
            assert!(blocked.command("directory", Fields::from([
                ("kind".into(), "cache".into()), ("migrate".into(), "true".into()),
                ("path".into(), target.path().to_string_lossy().into()),
            ])).await.is_err());
        }
        drop(blocked);
        assert!(cache.exists());
        assert!(log.exists());
    }
    first.close().await.unwrap();
    drop(first);
    let _released = Service::open(b.path().into()).unwrap();
    assert!(!log.exists());
}

#[tokio::test]
async fn export_rechecks_directory_and_lock_without_status_poll() {
    for replace_directory in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let mut app = Service::open(home.clone()).unwrap();
        app.command("init", Fields::new()).await.unwrap();
        let root = home.join("logs");
        let changed = if replace_directory { root.clone() } else { root.join(".gbf-flash-cache.lock") };
        if let Err(error) = std::fs::rename(&changed, home.join("detached")) {
            assert!(cfg!(windows), "{error}");
            continue;
        }
        if replace_directory { std::fs::create_dir(&root).unwrap(); }
        let lock = std::fs::File::create(root.join(".gbf-flash-cache.lock")).unwrap();
        lock.try_lock().unwrap();
        let log = root.join("other.jsonl");
        std::fs::write(&log, b"another instance").unwrap();
        let destination = temp.path().join("out.zip");
        let result = app.command("export", Fields::from([
            ("path".into(), destination.to_string_lossy().into())
        ])).await;
        assert!(result.is_err());
        assert!(!destination.exists());
        assert_eq!(std::fs::read(log).unwrap(), b"another instance");
    }
}
