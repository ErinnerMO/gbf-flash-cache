//! Regression checks for directory ownership, CDN date conditions and UDP source handling.
use bytes::Bytes;
use gbf_flash_cache_core::{
    cache::Cache,
    network::{Network, Request},
    service::{Fields, Service},
};
use http::{HeaderMap, Method};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
};

#[tokio::test]
async fn cdn_if_modified_since_returns_304_on_miss_and_hit() {
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = origin.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut s, _) = origin.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(s.read_u8().await.unwrap());
        }
        s.write_all(b"HTTP/1.1 200 OK\r\nLast-Modified: Mon, 21 Sep 2026 00:00:00 GMT\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody").await.unwrap();
        request
    });
    let home = tempfile::tempdir().unwrap();
    let cache = Cache::new(
        home.path().to_owned(),
        1048576,
        3600,
        Network::new(None, &[]).unwrap(),
    )
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        "if-modified-since",
        "Mon, 21 Sep 2026 00:00:00 GMT".parse().unwrap(),
    );
    let request = Request {
        method: Method::GET,
        url: format!("http://{address}/assets/a.js").parse().unwrap(),
        headers,
        body: Bytes::new(),
    };
    let miss = cache.get(request.clone(), false).await.unwrap();
    let sent = String::from_utf8(server.await.unwrap()).unwrap();
    let hit = cache.get(request, false).await.unwrap();
    assert!(!sent.to_ascii_lowercase().contains("if-modified-since"));
    assert_eq!(miss.reply.status, 304);
    assert_eq!(hit.reply.status, 304);
    assert!(miss.reply.body.is_empty());
    assert!(hit.reply.body.is_empty());
    cache.close().await;
}

#[tokio::test]
async fn replaced_directory_cannot_clear_another_instances_cache() {
    let temp = tempfile::tempdir().unwrap();
    let common = temp.path().join("cache");
    let mut a =
        Service::open_with_directories(temp.path().join("a"), Some(common.clone()), None).unwrap();
    if let Err(error) = std::fs::rename(&common, temp.path().join("detached-cache")) {
        // Windows can reject moving a directory containing the locked file.
        // Verify the exclusion outcome instead of pretending a replacement occurred.
        assert!(cfg!(windows));
        // Windows may report access denied (5) or a sharing violation (32).
        assert!(matches!(error.raw_os_error(), Some(5 | 32)), "{error}");
        assert!(common.is_dir());
        assert!(!temp.path().join("detached-cache").exists());
        let sentinel = common.join("owned-by-a.gfc");
        std::fs::write(&sentinel, b"owned by a").unwrap();
        let mut b = Service::open_with_directories(temp.path().join("b"), Some(common), None).unwrap();
        assert!(b.command("clear", Fields::new()).await.is_err());
        assert!(sentinel.exists());
        return;
    }
    std::fs::create_dir(&common).unwrap();
    let mut b =
        Service::open_with_directories(temp.path().join("b"), Some(common.clone()), None).unwrap();
    let sa = a.command("status", Fields::new()).await.unwrap();
    let sb = b.command("status", Fields::new()).await.unwrap();
    assert_ne!(sa["directoryWarning"], "");
    assert_eq!(sb["directoryWarning"], "");
    std::fs::write(common.join("other-instance.gfc"), b"owned by b").unwrap();
    assert!(a.command("clear", Fields::new()).await.is_err());
    assert!(common.join("other-instance.gfc").exists());
    b.close().await.unwrap();
    drop(b);
    a.command("stop", Fields::new()).await.unwrap();
    assert_eq!(
        a.command("status", Fields::new()).await.unwrap()["directoryWarning"],
        ""
    );
    a.command("clear", Fields::new()).await.unwrap();
    a.close().await.unwrap();
}

#[tokio::test]
async fn socks_udp_rejects_wrong_source_for_fixed_target() {
    for fixed in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let relay = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = relay.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut control, _) = listener.accept().await.unwrap();
            let mut greeting = [0; 3];
            control.read_exact(&mut greeting).await.unwrap();
            control.write_all(&[5, 0]).await.unwrap();
            let mut assoc = [0; 10];
            control.read_exact(&mut assoc).await.unwrap();
            let mut answer = vec![5, 0, 0, 1, 127, 0, 0, 1];
            answer.extend(port.to_be_bytes());
            control.write_all(&answer).await.unwrap();
            let mut buf = [0; 100];
            let (_, peer) = relay.recv_from(&mut buf).await.unwrap();
            let mut wrong = vec![0, 0, 0, 1, 203, 0, 113, 99, 0, 54];
            wrong.extend(b"wrong-origin");
            relay.send_to(&wrong, peer).await.unwrap();
            tokio::time::sleep(Duration::from_millis(500)).await;
        });
        let route = gbf_flash_cache_core::udp::Route::Socks {
            host: "127.0.0.1".into(),
            port: address.port(),
            user: String::new(),
            password: String::new(),
        };
        let mut transport = route.open(Some("192.0.2.1:53")).await.unwrap();
        transport.send(b"question").await.unwrap();
        let mut bytes = [0; 100];
        if fixed {
            assert!(transport.recv(&mut bytes).await.is_err());
        } else {
            let (n, source) = transport.recv_from(&mut bytes).await.unwrap();
            assert_eq!(source, "203.0.113.99:54");
            assert_eq!(&bytes[..n], b"wrong-origin");
        }
        server.await.unwrap();
    }
}

#[cfg(unix)]
#[test]
fn migration_aborts_if_source_is_replaced_after_copy_starts() {
    use std::{fs, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::Instant};
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let mut service = Service::open(home.clone()).unwrap();
    let source = home.join("cache");
    let replacement = temp.path().join("replacement");
    fs::create_dir(&replacement).unwrap();
    for i in 0..1000 {
        fs::write(source.join(format!("{i}.gfc")), b"original").unwrap();
        fs::write(replacement.join(format!("{i}.gfc")), b"other owner").unwrap();
    }
    let replacement_lock = fs::File::create(replacement.join(".gbf-flash-cache.lock")).unwrap();
    replacement_lock.try_lock().unwrap();
    let target = temp.path().join("gbf-flash-cache-cache");
    let args = Fields::from([
        ("kind".into(), "cache".into()), ("migrate".into(), "true".into()),
        ("path".into(), temp.path().to_string_lossy().into()),
    ]);
    let committed = Arc::new(AtomicBool::new(false));
    let flag = committed.clone();
    let task = std::thread::spawn(move || service.change_directory(&args, |_| {
        flag.store(true, Ordering::SeqCst); Ok(())
    }));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !fs::read_dir(&target).is_ok_and(|entries| entries.flatten().any(|e| e.path().extension().is_some_and(|s| s == "gfc"))) {
        assert!(!task.is_finished(), "migration finished before replacement");
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    fs::rename(&source, temp.path().join("original")).unwrap();
    fs::rename(&replacement, &source).unwrap();
    assert!(task.join().unwrap().is_err());
    assert!(!committed.load(Ordering::SeqCst));
    assert!(!fs::read_dir(target).unwrap().flatten().any(|e| e.path().extension().is_some_and(|s| s == "gfc")));
    assert_eq!(fs::read(source.join("0.gfc")).unwrap(), b"other owner");
}
