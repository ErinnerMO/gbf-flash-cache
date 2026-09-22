use bytes::Bytes;
use gbf_flash_cache_core::{
    cache::Cache,
    certificates::Authority,
    engine::Engine,
    gateway::Gateway,
    network::{Network, Request},
    tunnel::Route,
};
use http::{HeaderMap, Method};
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn};
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::{
    collections::HashMap,
    io,
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_rustls::{rustls, TlsAcceptor};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use url::Url;

#[derive(Default)]
struct State {
    counts: Mutex<HashMap<String, u64>>,
    connections: AtomicU64,
    headers: Mutex<HeaderMap>,
    h2: AtomicU64,
    last_request: Mutex<serde_json::Value>,
    gate: tokio::sync::Notify,
}
impl State {
    fn count(&self, path: &str) -> u64 {
        *self.counts.lock().unwrap().get(path).unwrap_or(&0)
    }
}
struct Origin {
    address: SocketAddr,
    root: Vec<u8>,
    state: Arc<State>,
    cancel: CancellationToken,
    tasks: TaskTracker,
    _home: tempfile::TempDir,
}
impl Origin {
    async fn new(tls: bool) -> Self { Self::with_h2(tls, true).await }
    async fn with_h2(tls: bool, h2: bool) -> Self {
        let home = tempfile::tempdir().unwrap();
        let ca = Authority::open(home.path()).unwrap();
        let leaf = ca.leaf("127.0.0.1").unwrap();
        let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.certificate.into(), ca.certificate.clone().into()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(leaf.key).into(),
        )
        .unwrap();
        config.alpn_protocols = if h2 { vec![b"h2".to_vec(), b"http/1.1".to_vec()] } else { vec![b"http/1.1".to_vec()] };
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::new(State::default());
        let cancel = CancellationToken::new();
        let tasks = TaskTracker::new();
        let child_tasks = tasks.clone();
        let stop = cancel.clone();
        let observed = state.clone();
        tasks.spawn(async move {loop {
            let stream=tokio::select!{_=stop.cancelled()=>break,accepted=listener.accept()=>accepted.unwrap().0};let state=observed.clone();state.connections.fetch_add(1,Ordering::Relaxed);let acceptor=acceptor.clone();let stop=stop.clone();
            child_tasks.spawn(async move {tokio::select!{_=stop.cancelled()=>{},_=async {
                let service=service_fn(move |request|respond(request,state.clone()));
                if tls {let Ok(stream)=acceptor.accept(stream).await else {return;};if stream.get_ref().1.alpn_protocol()==Some(b"h2") {let _=hyper::server::conn::http2::Builder::new(TokioExecutor::new()).serve_connection(TokioIo::new(stream),service).await;} else {let _=hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream),service).with_upgrades().await;}}
                else {let _=hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream),service).with_upgrades().await;}
            }=>{}}});
        }});
        Self {
            address,
            root: ca.certificate,
            state,
            cancel,
            tasks,
            _home: home,
        }
    }
    fn url(&self, path: &str) -> Url {
        Url::parse(&format!("https://{}{path}", self.address)).unwrap()
    }
    async fn close(&self) {
        self.cancel.cancel();
        self.tasks.close();
        self.tasks.wait().await;
    }
}
fn test_body(bytes: Bytes) -> http_body_util::combinators::UnsyncBoxBody<Bytes, Box<dyn std::error::Error + Send + Sync>> {
    Full::new(bytes).map_err(|never| match never {}).boxed_unsync()
}
struct FrameBody(tokio::sync::mpsc::Receiver<Result<hyper::body::Frame<Bytes>, Box<dyn std::error::Error + Send + Sync>>>);
impl hyper::body::Body for FrameBody {
    type Data = Bytes;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    fn poll_frame(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>)
        -> std::task::Poll<Option<Result<hyper::body::Frame<Bytes>, Self::Error>>> { self.0.poll_recv(cx) }
}
async fn respond(
    mut request: http::Request<Incoming>,
    state: Arc<State>,
) -> io::Result<http::Response<http_body_util::combinators::UnsyncBoxBody<Bytes, Box<dyn std::error::Error + Send + Sync>>>> {
    let raw_target = request.uri().path_and_query().unwrap().to_string();
    let path = request.uri().path().to_owned();
    *state
        .counts
        .lock()
        .unwrap()
        .entry(path.clone())
        .or_default() += 1;
    if request.version() == http::Version::HTTP_2 {
        state.h2.fetch_add(1, Ordering::Relaxed);
    }
    if path == "/assets/overflow-stream.png" || path == "/assets/overflow-fail.png" {
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        let fail = path.ends_with("fail.png");
        tokio::spawn(async move {
            for _ in 0..17 {
                if sender.send(Ok(hyper::body::Frame::data(Bytes::from(vec![42; 1024 * 1024])))).await.is_err() { return; }
            }
            state.gate.notified().await;
            if fail {
                let _ = sender.send(Err(io::Error::other("synthetic body failure").into())).await;
            } else {
                let _ = sender.send(Ok(hyper::body::Frame::data(Bytes::from_static(b"tail")))).await;
            }
        });
        return Ok(http::Response::builder().header("content-type", "image/png")
            .body(FrameBody(receiver).boxed_unsync()).unwrap());
    }
    if path == "/slow-stream" {
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        tokio::spawn(async move {
            for part in [b"first".as_slice(), b"second", b"third"] {
                if part != b"first" { tokio::time::sleep(Duration::from_secs(20)).await; }
                if sender.send(Ok(hyper::body::Frame::data(Bytes::copy_from_slice(part)))).await.is_err() { return; }
            }
            let mut trailers = HeaderMap::new();
            trailers.insert("x-checksum", "synthetic-checksum".parse().unwrap());
            let _ = sender.send(Ok(hyper::body::Frame::trailers(trailers))).await;
        });
        return Ok(http::Response::builder().header("trailer", "x-checksum")
            .body(FrameBody(receiver).boxed_unsync()).unwrap());
    }
    if path == "/stream" {
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        tokio::spawn(async move {
            sender.send(Ok(hyper::body::Frame::data(Bytes::from_static(b"first")))).await.unwrap();
            state.gate.notified().await;
            sender.send(Ok(hyper::body::Frame::data(Bytes::from_static(b"second")))).await.unwrap();
            let mut trailers = HeaderMap::new();
            trailers.insert("x-checksum", "synthetic-checksum".parse().unwrap());
            sender.send(Ok(hyper::body::Frame::trailers(trailers))).await.unwrap();
        });
        return Ok(http::Response::builder().header("trailer", "x-checksum")
            .body(FrameBody(receiver).boxed_unsync()).unwrap());
    }
    if path.starts_with("/review") || path == "*" {
        let method = request.method().to_string();
        let target = request.uri().path_and_query().unwrap().to_string();
        let te = request.headers().get("te").map(|value| value.to_str().unwrap().to_owned());
        let mut headers: Vec<_> = request.headers().iter().filter(|(name, _)|
            !["host", "connection", "content-length", "transfer-encoding", "te"].contains(&name.as_str()))
            .map(|(k,v)| (k.to_string(), v.as_bytes().to_vec())).collect();
        headers.sort_by(|a,b| a.0.cmp(&b.0));
        let host = request.uri().authority().map(|v| v.as_str()).or_else(|| request.headers().get("host").and_then(|v| v.to_str().ok())).unwrap().to_owned();
        let body = request.into_body().collect().await.map_err(io::Error::other)?;
        let trailers: Vec<_> = body.trailers().into_iter().flat_map(|h| h.iter())
            .map(|(k,v)| (k.to_string(), v.as_bytes().to_vec())).collect();
        let bytes = body.to_bytes();
        let echo = serde_json::json!({"method":method,"target":target,"te":te,
            "headers":headers,"host":host,"length":bytes.len(),"digest":gbf_flash_cache_core::hash(&bytes),"trailers":trailers});
        *state.last_request.lock().unwrap() = echo.clone();
        return Ok(http::Response::builder().header("content-type", "application/json")
            .header("set-cookie", "a=1").header("set-cookie", "b=2")
            .body(test_body(Bytes::from(echo.to_string()))).unwrap());
    }
    if path.starts_with("//") {
        let method = request.method().to_string();
        let target = request.uri().path_and_query().unwrap().to_string();
        let host = request.uri().authority().map(|v| v.as_str()).or_else(|| request.headers().get("host").and_then(|v| v.to_str().ok())).unwrap().to_owned();
        let body = request.into_body().collect().await.unwrap().to_bytes();
        return Ok(http::Response::new(test_body(Bytes::from(
            serde_json::json!({"method": method, "target": target, "host": host,
                "body": String::from_utf8(body.to_vec()).unwrap()}).to_string(),
        ))));
    }
    if path == "/socket" {
        let upgrade = hyper::upgrade::on(&mut request);
        tokio::spawn(async move {
            if let Ok(stream) = upgrade.await {
                let mut stream = TokioIo::new(stream);
                let mut bytes = [0; 4];
                if stream.read_exact(&mut bytes).await.is_ok() {
                    let _ = stream.write_all(&bytes).await;
                }
            }
        });
        return Ok(http::Response::builder()
            .status(101)
            .header("connection", "upgrade")
            .header("upgrade", "test-echo")
            .body(test_body(Bytes::new()))
            .unwrap());
    }
    if path == "/api/fail" {
        return Err(io::Error::other("synthetic disconnect"));
    }
    if path.starts_with("/assets/blocked") || path == "/wait-headers" {
        state.gate.notified().await;
    }
    if path == "/assets/slow.png" {
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    let mut response = http::Response::builder()
        .header("content-type", "image/png")
        .header("etag", "\"v1\"");
    let mut body = if path.starts_with("/assets/raw/") { Bytes::from(raw_target) } else { Bytes::from_static(b"original bytes") };
    if path.starts_with("/assets/host-") {
        body = Bytes::from(request.uri().authority().map(|a| a.as_str())
            .or_else(|| request.headers().get("host").and_then(|h| h.to_str().ok())).unwrap().to_owned());
    }
    if request
        .headers()
        .get("if-none-match")
        .is_some_and(|v| v == "\"v1\"")
    {
        response = response.status(304);
        body = Bytes::new();
    }
    if path == "/assets/no-store.png" {
        response = response.header("cache-control", "no-store");
    }
    if path == "/assets/vary.png" {
        response = response.header("vary", "Accept-Language");
        body = request
            .headers()
            .get("accept-language")
            .map(|v| Bytes::copy_from_slice(v.as_bytes()))
            .unwrap_or_default();
    }
    if path == "/api" {
        body = request
            .body_mut()
            .collect()
            .await
            .map_err(io::Error::other)?
            .to_bytes();
        *state.headers.lock().unwrap() = request.headers().clone();
    }
    if path == "/api/503" || path == "/assets/error.png" {
        response = response.status(503).header("retry-after", "0");
    }
    if path.ends_with("missing.png") { response = response.status(404); }
    if path.ends_with("gone.png") { response = response.status(410); }
    if path == "/redirect" {
        response = response.status(302).header("location", "/api");
    }
    if path == "/assets/123/js/source.js" {
        response = response.header("content-type", "application/javascript");
        body = Bytes::from_static(b"var image='/assets/child.png';");
    }
    if path == "/compressed" {
        use std::io::Write;
        *state.headers.lock().unwrap() = request.headers().clone();
        let mut encoded = Vec::new();
        {
            let mut writer = brotli::CompressorWriter::new(&mut encoded, 4096, 1, 22);
            writer.write_all(b"var image='/assets/compressed-child.png';").unwrap();
        }
        response = http::Response::builder().header("content-type", "application/javascript").header("content-encoding", "br");
        body = Bytes::from(encoded);
    }
    if path == "/large" {
        body = Bytes::from(vec![42; 17 * 1024 * 1024]);
    }
    if path == "/assets/oversized.png" || path == "/assets/at-limit.png" {
        tokio::time::sleep(Duration::from_millis(100)).await;
        body = Bytes::from(vec![42; if path.contains("at-limit") { 16 } else { 17 } * 1024 * 1024]);
    }
    Ok(response.body(test_body(body)).unwrap())
}
fn request(url: Url) -> Request {
    Request {
        method: Method::GET,
        url: url.as_str().parse().unwrap(),
        headers: HeaderMap::new(),
        body: Bytes::new(),
    }
}
fn network(origin: &Origin) -> Network {
    Network::new(None, std::slice::from_ref(&origin.root)).unwrap()
}
async fn wait_count(state: &State, path: &str, count: u64) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while state.count(path) < count {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cache_network_singleflight_stale_vary_and_no_replay() {
    let origin = Origin::new(true).await;
    let directory = tempfile::tempdir().unwrap();
    let cache = Cache::new(
        directory.path().into(),
        8 * 1024 * 1024,
        1,
        network(&origin),
    )
    .unwrap();
    let req = request(origin.url("/assets/slow.png"));
    let (a, b) = tokio::join!(cache.get(req.clone(), true), cache.get(req.clone(), false));
    assert_eq!(a.unwrap().reply.body, b.unwrap().reply.body);
    assert_eq!(origin.state.count("/assets/slow.png"), 1);
    assert_eq!(cache.get(req.clone(), false).await.unwrap().state, "HIT");
    let mut conditional = req.clone();
    conditional
        .headers
        .insert("if-none-match", "W/\"v1\"".parse().unwrap());
    assert_eq!(
        cache.get(conditional, false).await.unwrap().reply.status,
        304
    );
    assert_eq!(origin.state.count("/assets/slow.png"), 1);
    tokio::time::sleep(Duration::from_millis(1050)).await;
    let start = std::time::Instant::now();
    assert_eq!(cache.get(req.clone(), false).await.unwrap().state, "HIT");
    assert!(start.elapsed() < Duration::from_millis(100));
    wait_count(&origin.state, "/assets/slow.png", 2).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        cache.get(req, false).await.unwrap().reply.body,
        "original bytes"
    );
    assert_eq!(origin.state.count("/assets/slow.png"), 2);
    for language in ["en", "ja", "ja"] {
        let mut req = request(origin.url("/assets/vary.png"));
        req.headers
            .insert("accept-language", language.parse().unwrap());
        assert_eq!(cache.get(req, false).await.unwrap().reply.body, language);
    }
    assert_eq!(origin.state.count("/assets/vary.png"), 2);
    for _ in 0..2 {
        cache
            .get(request(origin.url("/assets/no-store.png")), false)
            .await
            .unwrap();
    }
    assert_eq!(origin.state.count("/assets/no-store.png"), 2);
    let mut operation = request(origin.url("/api"));
    operation.method = Method::POST;
    operation.body = Bytes::from_static(b"unchanged operation");
    operation
        .headers
        .insert("cookie", "synthetic=test".parse().unwrap());
    operation
        .headers
        .insert("x-version", "123".parse().unwrap());
    assert_eq!(cache.pass(&operation).await.unwrap().body, operation.body);
    assert_eq!(
        origin.state.headers.lock().unwrap().get("cookie").unwrap(),
        "synthetic=test"
    );
    assert!(!origin
        .state
        .headers
        .lock()
        .unwrap()
        .keys()
        .any(|n| n.as_str().starts_with("x-gbf")));
    operation.url = origin.url("/api/503").as_str().parse().unwrap();
    assert_eq!(cache.pass(&operation).await.unwrap().status, 503);
    assert_eq!(origin.state.count("/api/503"), 1);
    operation.url = origin.url("/api/fail").as_str().parse().unwrap();
    assert!(cache.pass(&operation).await.is_err());
    assert_eq!(origin.state.count("/api/fail"), 1);
    assert_eq!(
        cache
            .pass(&request(origin.url("/redirect")))
            .await
            .unwrap()
            .status,
        302
    );
    assert!(origin.state.h2.load(Ordering::Relaxed) > 0);
    assert!(cache.memory_bytes() > 0);
    cache.close().await;
    let restarted = Cache::new(directory.path().into(), 0, 3600, network(&origin)).unwrap();
    assert_eq!(
        restarted
            .get(request(origin.url("/assets/slow.png")), false)
            .await
            .unwrap()
            .state,
        "HIT"
    );
    restarted.close().await;
    // A failed disk write must not discard a successful upstream response.
    let broken = directory.path().join("broken");
    let cache = Cache::new(broken.clone(), 0, 3600, network(&origin)).unwrap();
    let req = request(origin.url("/assets/disk-failure.png"));
    // Occupy the output filename without removing the held cache directory.
    let blocked = broken.join(format!("{}.gfc", gbf_flash_cache_core::hash(req.url.to_string().as_bytes())));
    std::fs::create_dir(&blocked).unwrap();
    let reply = cache
        .get(req, false)
        .await
        .unwrap();
    assert_eq!(reply.reply.status, 200);
    assert_eq!(reply.reply.body, "original bytes");
    assert_eq!(cache.counters.storage_failures.load(Ordering::Relaxed), 1);
    assert!(blocked.is_dir());
    cache.close().await;
    origin.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ingress_all_protocols_preload_transparent_stream_and_shutdown() {
    let origin = Origin::new(true).await;
    let plain = Origin::new(false).await;
    let directory = tempfile::tempdir().unwrap();
    let ca = Authority::open(&directory.path().join("ca")).unwrap();
    let cache = Cache::new(
        directory.path().join("cache"),
        8 * 1024 * 1024,
        3600,
        network(&origin),
    )
    .unwrap();
    let engine = Engine::new(cache.clone(), origin.url("/")).unwrap();
    let gateway = Gateway::start(
        0,
        engine,
        &ca,
        &[origin.url("/")],
        Route::new(None, std::slice::from_ref(&origin.root)).unwrap(),
    )
    .await
    .unwrap();
    for protocol in ["http", "https", "socks5h"] {
        let client = reqwest::Client::builder()
            .no_proxy()
            .add_root_certificate(reqwest::Certificate::from_der(&ca.certificate).unwrap())
            .proxy(reqwest::Proxy::all(format!("{protocol}://{}", gateway.address)).unwrap())
            .build()
            .unwrap();
        let response = client
            .get(origin.url("/assets/a.png"))
            .send()
            .await
            .unwrap_or_else(|e| panic!("{protocol}: {e:?}"));
        assert_eq!(response.status(), 200, "{protocol}");
        assert_eq!(
            response.bytes().await.unwrap(),
            "original bytes",
            "{protocol}"
        );
    }
    // Origin-form // is a path on the tunnel origin, never a new authority.
    for protocol in ["http", "https", "socks5h"] {
        let client = reqwest::Client::builder()
            .no_proxy()
            .add_root_certificate(reqwest::Certificate::from_der(&ca.certificate).unwrap())
            .proxy(reqwest::Proxy::all(format!("{protocol}://{}", gateway.address)).unwrap())
            .build().unwrap();
        for target in ["//rest/raid/auto_setting?_=123&t=123&uid=1",
            "///rest/raid/auto_setting?value=a%2Fb",
            "//elsewhere.invalid:443/path%2Fpart?value=%23%26"] {
            let body = r#"{"special_token":null,"value":2,"is_multi":false,"raid_id":1}"#;
            let response = client.post(origin.url(target))
                .header("content-type", "application/json").body(body).send().await.unwrap();
            assert_eq!(response.status(), 200, "{protocol} {target}");
            let echoed: serde_json::Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
            assert_eq!(echoed["method"], "POST");
            assert_eq!(echoed["target"], target);
            assert_eq!(echoed["host"], origin.address.to_string());
            assert_eq!(echoed["body"], body);
        }
    }
    let socks = tokio_socks::tcp::Socks4Stream::connect(
        gateway.address,
        ("127.0.0.1", origin.address.port()),
    )
    .await
    .unwrap()
    .into_inner();
    let tls = Route::new(None, std::slice::from_ref(&ca.certificate))
        .unwrap()
        .secure(socks, "127.0.0.1")
        .await
        .unwrap();
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let response = sender
        .send_request(
            http::Request::builder()
                .uri("/assets/a.png")
                .header("host", origin.address.to_string())
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "original bytes"
    );
    assert_eq!(origin.state.count("/assets/a.png"), 1);
    let client = reqwest::Client::builder()
        .no_proxy()
        .add_root_certificate(reqwest::Certificate::from_der(&ca.certificate).unwrap())
        .proxy(reqwest::Proxy::all(format!("http://{}", gateway.address)).unwrap())
        .build()
        .unwrap();
    client
        .get(origin.url("/assets/123/js/source.js"))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    wait_count(&origin.state, "/assets/child.png", 1).await;
    let response = client
        .get(format!("http://{}/large", plain.address))
        .send()
        .await
        .unwrap();
    assert_eq!(response.bytes().await.unwrap().len(), 17 * 1024 * 1024);
    // A non-inspected HTTPS origin keeps its own certificate through CONNECT.
    let outside = Origin::new(true).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .add_root_certificate(reqwest::Certificate::from_der(&outside.root).unwrap())
        .proxy(reqwest::Proxy::all(format!("http://{}", gateway.address)).unwrap())
        .build()
        .unwrap();
    assert_eq!(
        client
            .get(outside.url("/assets/a.png"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    // Raw HTTP upgrade survives forwarding and carries bytes in both directions.
    let mut stream = TcpStream::connect(gateway.address).await.unwrap();
    stream.write_all(format!("GET http://{}/socket HTTP/1.1\r\nHost: {}\r\nConnection: upgrade\r\nUpgrade: test-echo\r\n\r\n",plain.address,plain.address).as_bytes()).await.unwrap();
    let mut header = vec![];
    while !header.ends_with(b"\r\n\r\n") {
        header.push(stream.read_u8().await.unwrap());
    }
    assert!(header.starts_with(b"HTTP/1.1 101"));
    stream.write_all(b"ping").await.unwrap();
    let mut echo = [0; 4];
    stream.read_exact(&mut echo).await.unwrap();
    assert_eq!(&echo, b"ping");
    let address = gateway.address;
    tokio::time::timeout(Duration::from_secs(5), gateway.close())
        .await
        .unwrap();
    assert!(TcpStream::connect(address).await.is_err());
    outside.close().await;
    plain.close().await;
    origin.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_preload_promotes_and_api_pool_remains_available() {
    let origin = Origin::new(true).await;
    let directory = tempfile::tempdir().unwrap();
    let cache = Cache::new(directory.path().into(), 0, 3600, network(&origin)).unwrap();
    let mut tasks = vec![];
    for i in 0..2 {
        let c = cache.clone();
        let r = request(origin.url(&format!("/assets/blocked-{i}.png")));
        tasks.push(tokio::spawn(async move { c.get(r, true).await }));
    }
    for i in 0..2 {
        wait_count(&origin.state, &format!("/assets/blocked-{i}.png"), 1).await;
    }
    let req = request(origin.url("/assets/promoted.png"));
    let c = cache.clone();
    let r = req.clone();
    let preload = tokio::spawn(async move { c.get(r, true).await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while cache.pending_downloads() < 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(origin.state.count("/assets/promoted.png"), 0);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), cache.get(req, false))
            .await
            .unwrap()
            .unwrap()
            .reply
            .body,
        "original bytes"
    );
    assert_eq!(preload.await.unwrap().unwrap().reply.body, "original bytes");
    assert_eq!(origin.state.count("/assets/promoted.png"), 1);
    for i in 2..18 {
        let c = cache.clone();
        let r = request(origin.url(&format!("/assets/blocked-{i}.png")));
        tasks.push(tokio::spawn(async move { c.get(r, false).await }));
    }
    for i in 2..18 {
        wait_count(&origin.state, &format!("/assets/blocked-{i}.png"), 1).await;
    }
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(2),
            cache.pass(&request(origin.url("/api")))
        )
        .await
        .unwrap()
        .unwrap()
        .status,
        200
    );
    tokio::time::timeout(Duration::from_secs(3), cache.close())
        .await
        .unwrap();
    for task in tasks {
        assert!(task.await.unwrap().is_err());
    }
    origin.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_upstream_protocols_negotiate_h2_without_proxy_header_leakage() {
    let origin = Origin::new(true).await;
    let home = tempfile::tempdir().unwrap();
    let ca = Authority::open(&home.path().join("ca")).unwrap();
    let cache = Cache::new(home.path().join("cache"), 0, 3600, network(&origin)).unwrap();
    let engine = Engine::new(cache, origin.url("/")).unwrap();
    let proxy = Gateway::start(0, engine, &ca, &[], Route::new(None, &[]).unwrap())
        .await
        .unwrap();
    for protocol in ["http", "https", "socks4", "socks5"] {
        let network = Network::new(
            Some(&format!("{protocol}://{}", proxy.address)),
            &[origin.root.clone(), ca.certificate.clone()],
        )
        .unwrap();
        let mut req = request(origin.url("/api"));
        req.method = Method::POST;
        req.body = Bytes::from_static(b"unaltered");
        req.headers
            .insert("proxy-authorization", "do-not-leak".parse().unwrap());
        assert_eq!(
            network
                .fetch(&req, false)
                .await
                .unwrap_or_else(|e| panic!("{protocol}: {e}"))
                .body,
            "unaltered"
        );
        assert!(!origin
            .state
            .headers
            .lock()
            .unwrap()
            .contains_key("proxy-authorization"));
    }
    assert_eq!(origin.state.h2.load(Ordering::Relaxed), 4);
    proxy.close().await;
    origin.close().await;
}

#[tokio::test]
async fn upstream_failures_keep_stage_without_replaying() {
    let origin = Origin::new(true).await;
    let network = Network::new(None, &[]).unwrap();
    assert_eq!(network.fetch(&request(origin.url("/")), false).await.unwrap_err(), "upstream_tls_certificate");
    assert_eq!(origin.state.connections.load(Ordering::Relaxed), 1);
    origin.close().await;
    assert_eq!(network.fetch(&request(origin.url("/")), false).await.unwrap_err(), "upstream_connect");
}

#[test]
fn gbf_cdn_scope_is_explicit() {
    use gbf_flash_cache_core::{refs::ResourceRefs, CDN, CDN_HOSTS};
    let parser = ResourceRefs::new(CDN).unwrap();
    for host in CDN_HOSTS {
        assert!(parser.asset(&format!("https://{host}/assets/img/test.png")));
        assert!(!parser.asset(&format!("https://{host}/quest/data.json")));
        assert!(!parser.asset(&format!("https://{host}:8443/assets/img/test.png")));
    }
    assert!(!parser.asset("https://unrelated.akamaized.net/assets/img/test.png"));
    assert!(!parser.asset("https://gbf.akamaized.net.evil.example/assets/img/test.png"));
}

#[test]
fn mbga_cdn_is_cached_and_discovered_from_main_site() {
    use gbf_flash_cache_core::{refs::ResourceRefs, CDN, CDN_HOSTS};
    for shard in ["", "1", "2", "3", "4", "5"] {
        let host = format!("prd-game-a{shard}-gbf.akamaized.net");
        let resource = format!("https://{host}/assets/img/sp/touch_icon.png");
        let mut parser = ResourceRefs::new(CDN).unwrap();
        assert!(parser.asset(&resource), "Mobage CDN resource must be cacheable");
        assert!(CDN_HOSTS.contains(&host.as_str()), "gateway must intercept Mobage CDN");
        let html = format!(r#"<img src="{resource}">"#);
        let refs = parser.parse("https://gbf.game.mbga.jp/", "text/html", "", html.as_bytes());
        assert!(refs.contains_key(&resource), "main-site resources must enter real-time preload");
        assert!(!parser.asset(&format!("https://{host}/rest/data.json")));
        assert!(!parser.asset(&format!("https://{host}.evil.example/assets/img/a.png")));
    }
    let parser = ResourceRefs::new(CDN).unwrap();
    assert!(!parser.asset("https://prd-game-a6-gbf.akamaized.net/assets/img/a.png"));
}

#[tokio::test]
async fn preload_counter_excludes_cache_reads_and_browser_downloads() {
    let origin = Origin::new(true).await;
    let home = tempfile::tempdir().unwrap();
    let cache = Cache::new(home.path().join("cache"), 1048576, 3600, network(&origin)).unwrap();
    let resource = request(origin.url("/assets/preloaded.png"));
    cache.get(resource.clone(), true).await.unwrap();
    assert_eq!(cache.counters.preloaded.load(Ordering::Relaxed), 1);
    cache.get(resource.clone(), true).await.unwrap();
    assert_eq!(cache.counters.preloaded.load(Ordering::Relaxed), 1);
    assert_eq!(cache.counters.hits.load(Ordering::Relaxed), 0);
    cache.get(resource, false).await.unwrap();
    assert_eq!(cache.counters.hits.load(Ordering::Relaxed), 1);
    cache.get(request(origin.url("/assets/demand.png")), false).await.unwrap();
    assert_eq!(cache.counters.preloaded.load(Ordering::Relaxed), 1);
    cache.close().await;
    origin.close().await;
}

#[tokio::test]
async fn resource_requests_exclude_main_site_but_errors_include_it() {
    let origin = Origin::new(true).await;
    let home = tempfile::tempdir().unwrap();
    let cache = Cache::new(home.path().into(), 1048576, 3600, network(&origin)).unwrap();
    let engine = Engine::new(cache.clone(), origin.url("/")).unwrap();
    for path in ["/", "/api", "/api/503", "/outside.png"] {
        engine.request(request(origin.url(path))).await.unwrap();
    }
    assert_eq!(cache.counters.requests.load(Ordering::Relaxed), 0);
    assert_eq!(cache.counters.failures.load(Ordering::Relaxed), 1);
    cache.get(request(origin.url("/assets/preload.png")), true).await.unwrap();
    engine.request(request(origin.url("/assets/preload.png"))).await.unwrap();
    engine.request(request(origin.url("/assets/demand.png"))).await.unwrap();
    engine.request(request(origin.url("/assets/error.png"))).await.unwrap();
    assert_eq!(cache.counters.requests.load(Ordering::Relaxed), 3);
    assert_eq!(cache.counters.preloaded.load(Ordering::Relaxed), 1);
    assert_eq!(cache.counters.hits.load(Ordering::Relaxed), 1);
    assert_eq!(cache.counters.failures.load(Ordering::Relaxed), 2);
    engine.close().await;
    origin.close().await;
}

#[tokio::test]
async fn tls_uses_only_supplied_trust_roots() {
    let origin = Origin::new(true).await;
    let route = Route::new(None, &[]).unwrap();
    let stream = TcpStream::connect(origin.address).await.unwrap();
    assert!(matches!(route.secure(stream, "127.0.0.1").await, Err(e) if e == "tls_certificate"));
    let route = Route::new(None, std::slice::from_ref(&origin.root)).unwrap();
    let stream = TcpStream::connect(origin.address).await.unwrap();
    assert!(route.secure(stream, "127.0.0.1").await.is_ok());
    origin.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ip_ingress_uses_tls_name_for_cache_and_preserves_other_tls() {
    let origin = Origin::new(true).await;
    let home = tempfile::tempdir().unwrap();
    let ca = Authority::open(&home.path().join("ca")).unwrap();
    let mut url = origin.url("/");
    url.set_host(Some("localhost")).unwrap();
    let trace = gbf_flash_cache_core::trace::Trace::open(&home.path().join("ingress.jsonl")).unwrap();
    let cache = Cache::new(home.path().join("cache"), 1048576, 3600, network(&origin).with_trace(trace.clone())).unwrap();
    let engine = Engine::new(cache.clone(), url.clone()).unwrap();
    let gateway = Gateway::start(0, engine, &ca, &[url], Route::new(None, &[]).unwrap()).await.unwrap();
    for protocol in ["http", "socks4", "socks5", "unmatched"] {
        let stream = match protocol {
            "socks4" => tokio_socks::tcp::Socks4Stream::connect(gateway.address, origin.address).await.unwrap().into_inner(),
            "socks5" => tokio_socks::tcp::Socks5Stream::connect(gateway.address, origin.address).await.unwrap().into_inner(),
            _ => {
                let mut stream = TcpStream::connect(gateway.address).await.unwrap();
                stream.write_all(format!("CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n", origin.address, origin.address).as_bytes()).await.unwrap();
                let mut reply = Vec::new();
                while !reply.ends_with(b"\r\n\r\n") { reply.push(stream.read_u8().await.unwrap()); }
                assert!(reply.starts_with(b"HTTP/1.1 200"));
                stream
            }
        };
        // Unknown/no SNI must keep the real server's certificate and unmodified bytes.
        let (root, host) = if protocol == "unmatched" { (&origin.root, "127.0.0.1") } else { (&ca.certificate, "localhost") };
        let tls = Route::new(None, std::slice::from_ref(root)).unwrap().secure(stream, host).await.unwrap();
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls)).await.unwrap();
        tokio::spawn(async move { let _ = connection.await; });
        let response = sender.send_request(http::Request::builder().uri("/assets/a.png").header("host", format!("localhost:{}", origin.address.port()))
            .body(Full::new(Bytes::new())).unwrap()).await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.into_body().collect().await.unwrap().to_bytes(), "original bytes");
    }
    assert_eq!(cache.counters.requests.load(Ordering::Relaxed), 1);
    assert_eq!(cache.counters.hits.load(Ordering::Relaxed), 2);
    assert_eq!(origin.state.count("/assets/a.png"), 2); // First cache fill + unmatched TLS relay.
    gateway.close().await;
    origin.close().await;
    trace.close().await;
    let text = std::fs::read_to_string(home.path().join("ingress.jsonl")).unwrap();
    let rows: Vec<serde_json::Value> = text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    for (protocol, action) in [("CONNECT","accepted"),("SOCKS4","accepted"),("SOCKS5","accepted"),("TLS","sni"),("TLS","inspect"),("TCP","relay")] {
        assert!(rows.iter().any(|r| r["event"] == "INGRESS" && r["protocol"] == protocol && r["action"] == action), "{protocol} {action}");
    }
    assert!(!text.contains("original bytes"));

}

// Raw HTTP client: URL libraries in a test client must not normalize the fixture first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn main_site_forwarding_preserves_target_method_body_and_headers() {
    for h2 in [false, true] {
        let origin = Origin::with_h2(true, h2).await;
        let home = tempfile::tempdir().unwrap();
        let ca = Authority::open(&home.path().join("ca")).unwrap();
        let cache = Cache::new(home.path().join("cache"), 1048576, 3600, network(&origin)).unwrap();
        let gateway = Gateway::start(0, Engine::new(cache.clone(), origin.url("/")).unwrap(), &ca,
            &[origin.url("/")], Route::new(None, std::slice::from_ref(&origin.root)).unwrap()).await.unwrap();
        let large = Bytes::from(vec![42; 17 * 1024 * 1024]);
        let body = Bytes::from_static(b"{\"value\":2,\"token\":null}\x00\xff");
        let cases = [
            ("POST", "/review/a/../b?x=1", body.clone()),
            ("POST", r"/review/a\b?x=%2f&x=%2F", body.clone()),
            ("POST", "/review/%2e%2e/b?x=1", body.clone()),
            ("POST", "/review/query?x='quoted'&x=a%2Fb&empty=&x=+", body.clone()),
            ("PATCH", "/review/%2Fpart?x=%2523", body.clone()),
            ("GET", "/review/body", body.clone()),
            ("HEAD", "/review/head", body.clone()),
            ("OPTIONS", "*", Bytes::new()),
            ("DELETE", "/review/delete", body.clone()),
            ("CUSTOM", "/review/custom", body.clone()),
            ("POST", "/review/large", large.clone()),
        ];
        for protocol in ["direct", "http", "https", "socks4", "socks5"] {
            let stream: gbf_flash_cache_core::tunnel::BoxStream = match protocol {
                "direct" => Box::new(TcpStream::connect(origin.address).await.unwrap()),
                "socks4" => Box::new(tokio_socks::tcp::Socks4Stream::connect(gateway.address, origin.address).await.unwrap().into_inner()),
                "socks5" => Box::new(tokio_socks::tcp::Socks5Stream::connect(gateway.address, origin.address).await.unwrap().into_inner()),
                _ => {
                    let tcp = TcpStream::connect(gateway.address).await.unwrap();
                    let mut io: gbf_flash_cache_core::tunnel::BoxStream = if protocol == "https" {
                        Box::new(Route::new(None, std::slice::from_ref(&ca.certificate)).unwrap().secure(tcp, "127.0.0.1").await.unwrap())
                    } else { Box::new(tcp) };
                    io.write_all(format!("CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n", origin.address, origin.address).as_bytes()).await.unwrap();
                    let mut reply = Vec::new();
                    while !reply.ends_with(b"\r\n\r\n") { reply.push(io.read_u8().await.unwrap()); }
                    assert!(reply.starts_with(b"HTTP/1.1 200"));
                    io
                }
            };
            let root = if protocol == "direct" { &origin.root } else { &ca.certificate };
            let tls = Route::new(None, std::slice::from_ref(root)).unwrap().secure(stream, "127.0.0.1").await.unwrap();
            let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls)).await.unwrap();
            let driver = tokio::spawn(async move { connection.await.unwrap(); });
            for (index, (method, target, bytes)) in cases.iter().enumerate() {
                let te = ["trailers", "Trailers", "TRAILERS"][index % 3];
                let request = http::Request::builder().method(*method).uri(*target)
                    .header("host", origin.address.to_string())
                    .header("te", te)
                    .header("content-type", "application/json")
                    .header("cookie", "synthetic=1; synthetic=2")
                    .header("authorization", "Bearer synthetic")
                    .header("x-version", "123")
                    .header("origin", "https://example.invalid")
                    .header("referer", "https://example.invalid/?a=%2f")
                    .header("accept-encoding", "gzip, br")
                    .header("x-duplicate", "first").header("x-duplicate", "second")
                    .body(Full::new(bytes.clone())).unwrap();
                let mut expected: Vec<_> = request.headers().iter().filter(|(n,_)| !["host", "te"].contains(&n.as_str()))
                    .map(|(k,v)| (k.to_string(), v.as_bytes().to_vec())).collect();
                expected.sort_by(|a,b| a.0.cmp(&b.0));
                sender.ready().await.unwrap();
                let response = sender.send_request(request).await.unwrap();
                assert_eq!(response.status(), 200, "{protocol} h2={h2} {method} {target}");
                assert_eq!(response.headers().get_all("set-cookie").iter().count(), 2);
                let received = response.into_body().collect().await.unwrap().to_bytes();
                let echo: serde_json::Value = if *method == "HEAD" {
                    assert!(received.is_empty()); origin.state.last_request.lock().unwrap().clone()
                } else { serde_json::from_slice(&received).unwrap() };
                assert_eq!(echo["target"], *target, "{protocol} h2={h2}");
                assert_eq!(echo["method"], *method);
                assert!(echo["te"].as_str().is_some_and(|value| value.eq_ignore_ascii_case("trailers")), "TE lost: {protocol} h2={h2} sent={te}");
                assert_eq!(echo["host"], origin.address.to_string());
                assert_eq!(echo["length"], bytes.len());
                assert_eq!(echo["digest"], gbf_flash_cache_core::hash(bytes));
                assert_eq!(echo["headers"], serde_json::to_value(expected).unwrap());
            }
            sender.ready().await.unwrap();
            let response = sender.send_request(http::Request::builder().uri("/large")
                .header("host", origin.address.to_string()).body(Full::new(Bytes::new())).unwrap()).await.unwrap();
            assert_eq!(response.status(), 200);
            assert_eq!(response.into_body().collect().await.unwrap().to_bytes(), large);
            drop(sender);
            driver.abort();
        }
        assert_eq!(cache.counters.requests.load(Ordering::Relaxed), 0);
        assert_eq!(cache.counters.failures.load(Ordering::Relaxed), 0);
        assert_eq!(origin.state.connections.load(Ordering::Relaxed), 2, "one direct + one pooled upstream connection");
        gateway.close().await;
        origin.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_host_preserves_virtual_host_and_bypasses_asset_cache() {
    for h2 in [true, false] {
        let origin = Origin::with_h2(true, h2).await;
        let home = tempfile::tempdir().unwrap();
        let ca = Authority::open(&home.path().join("ca")).unwrap();
        let cache = Cache::new(home.path().join("cache"), 1048576, 3600, network(&origin)).unwrap();
        let gateway = Gateway::start(0, Engine::new(cache.clone(), origin.url("/")).unwrap(), &ca,
            &[origin.url("/")], Route::new(None, std::slice::from_ref(&origin.root)).unwrap()).await.unwrap();
        let mut stream = TcpStream::connect(gateway.address).await.unwrap();
        stream.write_all(format!("CONNECT {} HTTP/1.1\r\nHost: {}\r\n\r\n", origin.address, origin.address).as_bytes()).await.unwrap();
        let mut reply = Vec::new();
        while !reply.ends_with(b"\r\n\r\n") { reply.push(stream.read_u8().await.unwrap()); }
        assert!(reply.starts_with(b"HTTP/1.1 200"));
        let tls = Route::new(None, std::slice::from_ref(&ca.certificate)).unwrap().secure(stream, "127.0.0.1").await.unwrap();
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls)).await.unwrap();
        let driver = tokio::spawn(async move { let _ = connection.await; });
        let explicit = "original.example:443";
        let target = "/review/a/../b?x=%2f&x=%2F&empty=";
        let bytes = Bytes::from_static(b"original\x00\xffbody");
        let response = sender.send_request(http::Request::builder().method("POST").uri(target)
            .header("host", explicit).body(Full::new(bytes.clone())).unwrap()).await.unwrap();
        assert_eq!(response.status(), 200);
        let echo: serde_json::Value = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(echo["host"], explicit, "h2 capable={h2}");
        assert_eq!(echo["target"], target);
        assert_eq!(echo["method"], "POST");
        assert_eq!(echo["digest"], gbf_flash_cache_core::hash(&bytes));
        assert_eq!(echo["length"], bytes.len());
        assert_eq!(origin.state.h2.load(Ordering::Relaxed), 0);
        let normal = origin.address.to_string();
        // Exercise both a warm target cache and a different Host arriving before the target.
        for (path, hosts) in [
            ("/assets/host-warm.png", [&*normal, explicit, &*normal, explicit]),
            ("/assets/host-cold.png", [explicit, &*normal, explicit, &*normal]),
        ] {
            for host in hosts {
                sender.ready().await.unwrap();
                let response = sender.send_request(http::Request::builder().uri(path)
                    .header("host", host).body(Full::new(Bytes::new())).unwrap()).await.unwrap();
                assert_eq!(response.status(), 200);
                assert_eq!(response.into_body().collect().await.unwrap().to_bytes().as_ref(), host.as_bytes());
            }
            assert_eq!(origin.state.count(path), 3, "only the matching Host can use the cache");
        }
        assert_eq!(origin.state.h2.load(Ordering::Relaxed), if h2 { 2 } else { 0 });
        drop(sender);
        driver.abort();
        gateway.close().await;
        origin.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn main_site_streaming_trailers_and_no_replay() {
    for h2 in [false, true] {
        let origin = Origin::with_h2(true, h2).await;
        let home = tempfile::tempdir().unwrap();
        let ca = Authority::open(&home.path().join("ca")).unwrap();
        let cache = Cache::new(home.path().join("cache"), 1048576, 3600, network(&origin)).unwrap();
        let gateway = Gateway::start(0, Engine::new(cache.clone(), origin.url("/")).unwrap(), &ca,
            &[origin.url("/")], Route::new(None, std::slice::from_ref(&origin.root)).unwrap()).await.unwrap();
        let stream = tokio_socks::tcp::Socks5Stream::connect(gateway.address, origin.address).await.unwrap().into_inner();
        let tls = Route::new(None, std::slice::from_ref(&ca.certificate)).unwrap().secure(stream, "127.0.0.1").await.unwrap();
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls)).await.unwrap();
        let driver = tokio::spawn(async move { let _ = connection.await; });
        let make = |path: &str, body| http::Request::builder().uri(path)
            .header("host", origin.address.to_string()).header("te", "trailers").body(body).unwrap();
        let mut trailers = HeaderMap::new();
        trailers.insert("x-checksum", "original-request-trailer".parse().unwrap());
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        tx.send(Ok(hyper::body::Frame::data(Bytes::from_static(b"original-body")))).await.unwrap();
        tx.send(Ok(hyper::body::Frame::trailers(trailers))).await.unwrap();
        drop(tx);
        let body = FrameBody(rx).boxed_unsync();
        let mut request = make("/review/trailers", body);
        *request.method_mut() = Method::POST;
        request.headers_mut().insert("trailer", "x-checksum".parse().unwrap());
        sender.ready().await.unwrap();
        let response = sender.send_request(request).await.unwrap();
        assert_eq!(response.status(), 200);
        let echo: serde_json::Value = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(echo["digest"], gbf_flash_cache_core::hash(b"original-body"));
        assert_eq!(echo["trailers"], serde_json::json!([["x-checksum", b"original-request-trailer".to_vec()]]));
        sender.ready().await.unwrap();
        let response = tokio::time::timeout(Duration::from_secs(2), sender.send_request(make("/stream", test_body(Bytes::new())))).await.unwrap().unwrap();
        assert_eq!(response.status(), 200);
        let mut body = response.into_body();
        let first = tokio::time::timeout(Duration::from_secs(2), body.frame()).await.unwrap().unwrap().unwrap().into_data().unwrap();
        assert_eq!(first, "first", "must forward before origin completes");
        origin.state.gate.notify_one();
        let remainder = body.collect().await.unwrap();
        assert_eq!(remainder.trailers().unwrap().get("x-checksum").unwrap(), "synthetic-checksum");
        assert_eq!(remainder.to_bytes(), "second");
        sender.ready().await.unwrap();
        let response = sender.send_request(make("/redirect", test_body(Bytes::new()))).await.unwrap();
        assert_eq!(response.status(), 302);
        assert_eq!(response.headers().get("location").unwrap(), "/api");
        response.into_body().collect().await.unwrap();
        assert_eq!(origin.state.count("/api"), 0);
        let mut request = make("/api/fail", test_body(Bytes::from_static(b"synthetic-action")));
        *request.method_mut() = Method::POST;
        sender.ready().await.unwrap();
        let response = sender.send_request(request).await.unwrap();
        assert_eq!(response.status(), 502);
        response.into_body().collect().await.unwrap();
        assert_eq!(origin.state.count("/api/fail"), 1, "never replay a failed operation");
        assert_eq!(cache.counters.failures.load(Ordering::Relaxed), 1);
        driver.abort(); gateway.close().await; origin.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cache_raw_targets_survive_hits_refresh_disk_and_logs() {
    for h2 in [false, true] {
        let origin = Origin::with_h2(true, h2).await;
        let home = tempfile::tempdir().unwrap();
        let ca = Authority::open(&home.path().join("ca")).unwrap();
        let trace = gbf_flash_cache_core::trace::Trace::open(&home.path().join("trace.jsonl")).unwrap();
        let cache = Cache::new(home.path().join("cache"), 1048576, 1, network(&origin).with_trace(trace.clone())).unwrap();
        let gateway = Gateway::start(0, Engine::new(cache.clone(), origin.url("/")).unwrap(), &ca,
            &[origin.url("/")], Route::new(None, std::slice::from_ref(&origin.root)).unwrap()).await.unwrap();
        let stream = tokio_socks::tcp::Socks5Stream::connect(gateway.address, origin.address).await.unwrap().into_inner();
        let tls = Route::new(None, std::slice::from_ref(&ca.certificate)).unwrap().secure(stream, "127.0.0.1").await.unwrap();
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls)).await.unwrap();
        let driver = tokio::spawn(async move { let _ = connection.await; });
        let paths = ["/assets/raw/a/../b.png?x='raw'", "/assets/raw/b.png?x=%27raw%27",
            "/assets/raw/%2e/b.png?x=%2f&x=%2F", r"/assets/raw/a\b.png?empty=&x=+"];
        for path in paths {
            for _ in 0..2 {
                sender.ready().await.unwrap();
                let reply = sender.send_request(http::Request::builder().uri(path)
                    .header("host", origin.address.to_string()).body(Full::new(Bytes::new())).unwrap()).await.unwrap();
                assert_eq!(reply.status(), 200);
                assert_eq!(reply.into_body().collect().await.unwrap().to_bytes(), path);
            }
        }
        assert_eq!(cache.counters.hits.load(Ordering::Relaxed), paths.len() as u64);
        assert_eq!(cache.counters.requests.load(Ordering::Relaxed), paths.len() as u64);
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let path = paths[0];
        sender.ready().await.unwrap();
        let reply = sender.send_request(http::Request::builder().uri(path)
            .header("host", origin.address.to_string()).body(Full::new(Bytes::new())).unwrap()).await.unwrap();
        assert_eq!(reply.into_body().collect().await.unwrap().to_bytes(), path);
        wait_count(&origin.state, "/assets/raw/a/../b.png", 2).await;
        // Wait for the background validation to finish before shutting down.
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(sender); driver.abort(); gateway.close().await; trace.close().await;
        let rows: Vec<serde_json::Value> = std::fs::read_to_string(home.path().join("trace.jsonl")).unwrap()
            .lines().map(|s| serde_json::from_str(s).unwrap()).collect();
        assert!(rows.iter().any(|r| r["event"] == "NETWORK_DONE" && r["status"] == 304));
        assert!(rows.iter().any(|r| r["event"] == "REQUEST" && r["path"] == "/assets/raw/a/../b.png"
            && r["query_hash"] == gbf_flash_cache_core::hash(b"x='raw'")));
        let disk = Cache::new(home.path().join("cache"), 0, 3600, network(&origin)).unwrap();
        // Raw keys must survive restart; no normalization may alias these entries.
        for path in paths {
            let key = gbf_flash_cache_core::hash(format!("{}{}", origin.url("/").as_str().trim_end_matches('/'), path).as_bytes());
            let entry = gbf_flash_cache_core::storage::Entry::read(std::fs::File::open(home.path().join("cache").join(format!("{key}.gfc"))).unwrap()).unwrap();
            assert_eq!(entry.body, path.as_bytes());
            let url = format!("{}{}", origin.url("/").as_str().trim_end_matches('/'), path).parse().unwrap();
            let hit = disk.get(Request { method: Method::GET, url, headers: HeaderMap::new(), body: Bytes::new() }, true).await.unwrap();
            assert_eq!(hit.state, "HIT");
            assert_eq!(hit.reply.body, path);
        }
        assert_eq!(disk.counters.requests.load(Ordering::Relaxed), 0);
        disk.close().await; origin.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plain_http_preserves_host_raw_target_and_trailers() {
    let origin = Origin::new(false).await;
    let home = tempfile::tempdir().unwrap();
    let ca = Authority::open(&home.path().join("ca")).unwrap();
    let cache = Cache::new(home.path().join("cache"), 0, 3600, network(&origin)).unwrap();
    let gateway = Gateway::start(0, Engine::new(cache, origin.url("/")).unwrap(), &ca, &[], Route::new(None, &[]).unwrap()).await.unwrap();
    let stream = TcpStream::connect(gateway.address).await.unwrap();
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();
    let driver = tokio::spawn(async move { let _ = connection.await; });
    let mut trailers = HeaderMap::new(); trailers.insert("x-checksum", "original".parse().unwrap());
    let (tx, rx) = tokio::sync::mpsc::channel(2);
    tx.send(Ok(hyper::body::Frame::data(Bytes::from_static(b"body")))).await.unwrap();
    tx.send(Ok(hyper::body::Frame::trailers(trailers))).await.unwrap(); drop(tx);
    let path = "/review/a/../b?x='raw'";
    sender.ready().await.unwrap();
    let response = sender.send_request(http::Request::builder().method("POST")
        .uri(format!("http://{}{path}", origin.address)).header("host", "synthetic.invalid")
        .header("trailer", "x-checksum").header("te", "trailers")
        .body(FrameBody(rx).boxed_unsync()).unwrap()).await.unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let echo: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(echo["target"], path);
    assert_eq!(echo["host"], "synthetic.invalid");
    assert_eq!(echo["trailers"], serde_json::json!([["x-checksum", b"original".to_vec()]]));
    sender.ready().await.unwrap();
    let response = sender.send_request(http::Request::builder().uri(format!("http://{}/stream", origin.address))
        .header("host", origin.address.to_string()).header("te", "trailers").body(test_body(Bytes::new())).unwrap()).await.unwrap();
    let mut body = response.into_body();
    assert_eq!(body.frame().await.unwrap().unwrap().into_data().unwrap(), "first");
    origin.state.gate.notify_one();
    let rest = body.collect().await.unwrap();
    assert_eq!(rest.trailers().unwrap()["x-checksum"], "synthetic-checksum");
    sender.ready().await.unwrap();
    let pending = sender.send_request(http::Request::builder().uri(format!("http://{}/wait-headers", origin.address))
        .header("host", origin.address.to_string()).body(test_body(Bytes::new())).unwrap());
    tokio::pin!(pending);
    assert!(tokio::time::timeout(Duration::from_secs(41), &mut pending).await.is_err(), "relay must not impose the old 40-second header timeout");
    origin.state.gate.notify_one();
    assert_eq!(tokio::time::timeout(Duration::from_secs(3), pending).await.unwrap().unwrap().status(), 200);
    driver.abort(); gateway.close().await; origin.close().await;
}

#[tokio::test]
async fn preload_404_is_not_a_network_failure_but_demand_and_other_errors_are() {
    let origin = Origin::new(true).await;
    let home = tempfile::tempdir().unwrap();
    let cache = Cache::new(home.path().join("cache"), 1048576, 3600, network(&origin)).unwrap();
    let missing = request(origin.url("/assets/missing.png"));
    assert_eq!(cache.get(missing.clone(), true).await.unwrap().reply.status, 404);
    assert_eq!(cache.counters.failures.load(Ordering::Relaxed), 0);
    assert_eq!(cache.get(missing, false).await.unwrap().reply.status, 404);
    assert_eq!(cache.counters.failures.load(Ordering::Relaxed), 1);
    cache.get(request(origin.url("/assets/error.png")), true).await.unwrap();
    assert_eq!(cache.counters.failures.load(Ordering::Relaxed), 2);
    let blocked = request(origin.url("/assets/blocked-missing.png"));
    let c = cache.clone(); let b = blocked.clone();
    let preload = tokio::spawn(async move { c.get(b, true).await.unwrap() });
    wait_count(&origin.state, "/assets/blocked-missing.png", 1).await;
    let demand = cache.get(blocked, false);
    tokio::pin!(demand);
    assert!(tokio::time::timeout(Duration::from_millis(30), &mut demand).await.is_err());
    origin.state.gate.notify_one();
    assert_eq!(demand.await.unwrap().reply.status, 404);
    assert_eq!(preload.await.unwrap().reply.status, 404);
    assert_eq!(cache.counters.failures.load(Ordering::Relaxed), 3);
    assert_eq!(cache.counters.preloaded.load(Ordering::Relaxed), 0);
    cache.close().await; origin.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_assets_stream_without_cache_or_failed_request_replay() {
    for h2 in [false, true] {
        let origin = Origin::with_h2(true, h2).await;
        let home = tempfile::tempdir().unwrap();
        let ca = Authority::open(&home.path().join("ca")).unwrap();
        let directory = home.path().join("cache");
        let cache = Cache::new(directory.clone(), 1048576, 3600, network(&origin)).unwrap();
        let gateway = Gateway::start(0, Engine::new(cache.clone(), origin.url("/")).unwrap(), &ca,
            &[origin.url("/")], Route::new(None, std::slice::from_ref(&origin.root)).unwrap()).await.unwrap();
        let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(10))
            .add_root_certificate(reqwest::Certificate::from_der(&ca.certificate).unwrap())
            .proxy(reqwest::Proxy::all(format!("http://{}", gateway.address)).unwrap()).build().unwrap();
        let get = || async {
            let response = client.get(origin.url("/assets/oversized.png")).send().await.unwrap();
            assert_eq!(response.status(), 200);
            let bytes = response.bytes().await.unwrap();
            assert_eq!(bytes.len(), 17 * 1024 * 1024);
            assert!(bytes.iter().all(|b| *b == 42));
        };
        // A coalesced response cannot share a live stream. Both clients must receive all bytes.
        tokio::join!(get(), get());
        assert_eq!(origin.state.count("/assets/oversized.png"), 2);
        let bypass = client.get(origin.url("/assets/oversized.png")).header("cookie", "test=1").send().await.unwrap();
        assert_eq!(bypass.status(), 200);
        assert_eq!(bypass.bytes().await.unwrap().len(), 17 * 1024 * 1024);
        assert_eq!(origin.state.count("/assets/oversized.png"), 3);
        let mut response = tokio::time::timeout(Duration::from_secs(5),
            client.get(origin.url("/assets/overflow-stream.png")).send()).await.unwrap().unwrap();
        assert_eq!(response.status(), 200, "headers must arrive before upstream EOF");
        let prefix = response.chunk().await.unwrap().unwrap();
        assert!(prefix.iter().all(|b| *b == 42));
        origin.state.gate.notify_one();
        let tail = response.bytes().await.unwrap();
        assert_eq!(prefix.len() + tail.len(), 17 * 1024 * 1024 + 4);
        assert!(tail.ends_with(b"tail"));
        assert!(tail[..tail.len()-4].iter().all(|b| *b == 42));
        assert_eq!(origin.state.count("/assets/overflow-stream.png"), 1);
        assert_eq!(cache.memory_bytes(), 0);
        assert!(!std::fs::read_dir(&directory).unwrap().any(|e| e.unwrap().path().extension().is_some_and(|x| x == "gfc")));
        assert_eq!(cache.counters.failures.load(Ordering::Relaxed), 0);
        for _ in 0..2 {
            let response = client.get(origin.url("/assets/at-limit.png")).send().await.unwrap();
            assert_eq!(response.status(), 200);
            assert_eq!(response.bytes().await.unwrap().len(), 16 * 1024 * 1024);
        }
        assert_eq!(origin.state.count("/assets/at-limit.png"), 1, "the cache boundary still caches");
        let response = client.get(origin.url("/assets/overflow-fail.png")).send().await.unwrap();
        assert_eq!(response.status(), 200);
        origin.state.gate.notify_one();
        assert!(response.bytes().await.is_err(), "incomplete body must remain an error");
        assert_eq!(origin.state.count("/assets/overflow-fail.png"), 1, "never replay a failed stream");
        assert_eq!(cache.counters.failures.load(Ordering::Relaxed), 1);
        gateway.close().await;
        origin.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn brotli_observation_discovers_resources_without_changing_wire_bytes() {
    let origin = Origin::new(true).await;
    let home = tempfile::tempdir().unwrap();
    let ca = Authority::open(&home.path().join("ca")).unwrap();
    let cache = Cache::new(home.path().join("cache"), 1048576, 3600, network(&origin)).unwrap();
    let gateway = Gateway::start(0, Engine::new(cache.clone(), origin.url("/")).unwrap(), &ca,
        &[origin.url("/")], Route::new(None, std::slice::from_ref(&origin.root)).unwrap()).await.unwrap();
    let direct = reqwest::Client::builder().no_proxy()
        .add_root_certificate(reqwest::Certificate::from_der(&origin.root).unwrap()).build().unwrap();
    let expected = direct.get(origin.url("/compressed")).send().await.unwrap().bytes().await.unwrap();
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(10))
        .add_root_certificate(reqwest::Certificate::from_der(&ca.certificate).unwrap())
        .proxy(reqwest::Proxy::all(format!("http://{}", gateway.address)).unwrap()).build().unwrap();
    let response = client.get(origin.url("/compressed")).header("accept-encoding", "gzip, deflate, br, zstd")
        .send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-encoding"], "br");
    assert_eq!(response.bytes().await.unwrap(), expected);
    assert_eq!(origin.state.headers.lock().unwrap()["accept-encoding"], "gzip, deflate, br, zstd");
    wait_count(&origin.state, "/assets/compressed-child.png", 1).await;
    gateway.close().await;
    origin.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gateway_rejects_lossy_requests_before_forwarding() {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(b"original # request content").unwrap();
    let compressed = encoder.finish().unwrap();
    for mode in 0..3 {
        let inspected = mode != 0;
        let origin = Origin::with_h2(inspected, mode == 2).await;
        let home = tempfile::tempdir().unwrap();
        let ca = Authority::open(&home.path().join("ca")).unwrap();
        let cache = Cache::new(home.path().join("cache"), 0, 3600, network(&origin)).unwrap();
        let origins = if inspected { vec![origin.url("/")] } else { Vec::new() };
        let gateway = Gateway::start(0, Engine::new(cache, origin.url("/")).unwrap(), &ca,
            &origins, Route::new(None, std::slice::from_ref(&origin.root)).unwrap()).await.unwrap();
        let target = |path: &str| if inspected { path.to_owned() } else { format!("http://{}{path}", origin.address) };
        let exchange = |wire: Vec<u8>| {
            let gateway = &gateway; let origin = &origin; let ca = &ca;
            async move {
                let mut stream: gbf_flash_cache_core::tunnel::BoxStream = if inspected {
                    let stream = tokio_socks::tcp::Socks5Stream::connect(gateway.address, origin.address).await.unwrap().into_inner();
                    Box::new(Route::new(None, std::slice::from_ref(&ca.certificate)).unwrap().secure(stream, "127.0.0.1").await.unwrap())
                } else { Box::new(TcpStream::connect(gateway.address).await.unwrap()) };
                // Split the request line across writes: validation must not depend on read boundaries.
                stream.write_all(&wire[..8]).await.unwrap();
                tokio::task::yield_now().await;
                stream.write_all(&wire[8..]).await.unwrap();
                let mut reply = Vec::new();
                tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut reply)).await.unwrap().unwrap();
                String::from_utf8(reply).unwrap()
            }
        };
        let chunked = {
            let mut body = format!("{:x}\r\n", compressed.len()).into_bytes();
            body.extend_from_slice(&compressed);
            body.extend_from_slice(b"\r\n0\r\n\r\n");
            body
        };
        // Content-Encoding remains supported with either ordinary HTTP body framing.
        for (framing, body) in [
            (format!("Content-Length: {}", compressed.len()), compressed.clone()),
            ("Transfer-Encoding: ChUnKeD".to_owned(), chunked.clone()),
        ] {
            let path = "/review/valid%23part?x=%23&x=%2523";
            let mut wire = format!("POST {} HTTP/1.1\r\nHost: {}\r\n{framing}\r\nContent-Encoding: gzip\r\nX-Note: #keep\r\nConnection: close\r\n\r\n", target(path), origin.address).into_bytes();
            wire.extend(body);
            let reply = exchange(wire).await;
            assert!(reply.starts_with("HTTP/1.1 200"), "mode={mode}: {reply}");
            let echo = origin.state.last_request.lock().unwrap().clone();
            assert_eq!(echo["target"], path);
            assert_eq!(echo["method"], "POST");
            assert_eq!(echo["digest"], gbf_flash_cache_core::hash(&compressed));
            assert!(echo["headers"].as_array().unwrap().contains(&serde_json::json!(["content-encoding", b"gzip".to_vec()])));
        }
        for (path, framing, body, status) in [
            ("/review/rejected?x=1#keep=2", "Content-Length: 3", b"abc".to_vec(), 400),
            ("/review/rejected#keep?x=2", "Content-Length: 3", b"abc".to_vec(), 400),
            ("/review/rejected", "Transfer-Encoding: gzip, chunked", chunked.clone(), 501),
            ("/review/rejected", "Transfer-Encoding: gzip\r\nTransfer-Encoding: chunked", chunked.clone(), 501),
        ] {
            for pipelined in [false, true] {
                let mut wire = Vec::new();
                if pipelined {
                    let payload = b"#keep\r\nPOST /not-a-request#fragment HTTP/1.1";
                    wire.extend(format!("POST {} HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\n\r\n", target("/review/first"), origin.address, payload.len()).as_bytes());
                    wire.extend_from_slice(payload);
                }
                wire.extend(format!("POST {} HTTP/1.1\r\nHost: {}\r\n{framing}\r\nConnection: close\r\n\r\n", target(path), origin.address).as_bytes());
                wire.extend_from_slice(&body);
                let reply = exchange(wire).await;
                assert_eq!(reply.starts_with("HTTP/1.1 200"), pipelined, "mode={mode}: {reply}");
                assert!(reply.contains(&format!("HTTP/1.1 {status}")), "mode={mode} path={path}: {reply}");
                assert_eq!(origin.state.count("/review/rejected"), 0, "rejected request reached upstream");
                if pipelined {
                    assert_eq!(origin.state.last_request.lock().unwrap()["digest"], gbf_flash_cache_core::hash(b"#keep\r\nPOST /not-a-request#fragment HTTP/1.1"));
                }
            }
        }
        assert_eq!(origin.state.h2.load(Ordering::Relaxed) > 0, mode == 2);
        gateway.close().await;
        origin.close().await;
    }
}

#[tokio::test]
async fn confirmed_missing_assets_invalidate_memory_and_disk() {
    use gbf_flash_cache_core::storage::{Entry, variant};
    let origin = Origin::new(true).await;
    for memory in [0, 1048576] {
        for (path, status, removed) in [
            ("/assets/missing.png", 404, true),
            ("/assets/gone.png", 410, true),
            ("/assets/error.png", 200, false),
        ] {
            let home = tempfile::tempdir().unwrap();
            let url = origin.url(path);
            let headers = vec![("content-type".to_owned(), "image/png".to_owned())];
            let entry = Entry { checked: 0, variant: variant(&headers, &Vec::new()), headers, body: b"old resource".to_vec() };
            let file = home.path().join(format!("{}.gfc", gbf_flash_cache_core::hash(url.as_str().as_bytes())));
            entry.write(std::fs::File::create(&file).unwrap()).unwrap();
            let cache = Cache::new(home.path().to_owned(), memory, 3600, network(&origin)).unwrap();
            // The request triggering background validation still receives its stale hit.
            assert_eq!(cache.get(request(url.clone()), false).await.unwrap().reply.body, "old resource");
            tokio::time::timeout(Duration::from_secs(5), async {
                while cache.pending_downloads() != 0 { tokio::time::sleep(Duration::from_millis(5)).await; }
            }).await.unwrap();
            assert_eq!(file.exists(), !removed);
            if removed { assert_eq!(cache.memory_bytes(), 0); }
            for _ in 0..2 {
                assert_eq!(cache.get(request(url.clone()), false).await.unwrap().reply.status.as_u16(), status);
            }
            cache.close().await;
            let restarted = Cache::new(home.path().to_owned(), memory, 3600, network(&origin)).unwrap();
            assert_eq!(restarted.get(request(url), false).await.unwrap().reply.status.as_u16(), status);
            restarted.close().await;
        }
    }
    origin.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forwarding_timeouts_release_all_slots_and_report_504_only_before_headers() {
    async fn check(h2: bool, body_stall: bool) {
        let origin = Origin::with_h2(true, h2).await;
        let home = tempfile::tempdir().unwrap();
        let ca = Authority::open(&home.path().join("ca")).unwrap();
        let cache = Cache::new(home.path().join("cache"), 0, 3600, network(&origin)).unwrap();
        let gateway = Gateway::start(0, Engine::new(cache.clone(), origin.url("/")).unwrap(), &ca,
            &[origin.url("/")], Route::new(None, std::slice::from_ref(&origin.root)).unwrap()).await.unwrap();
        let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(40))
            .add_root_certificate(reqwest::Certificate::from_der(&ca.certificate).unwrap())
            .proxy(reqwest::Proxy::all(format!("http://{}", gateway.address)).unwrap()).build().unwrap();
        let path = if body_stall { "/stream" } else { "/wait-headers" };
        let mut stalled = tokio::task::JoinSet::new();
        for _ in 0..32 {
            let client = client.clone();
            let url = origin.url(path);
            stalled.spawn(async move {
                let mut response = client.get(url).send().await.unwrap();
                if body_stall {
                    assert_eq!(response.status(), 200, "already forwarded status must stay 200");
                    assert_eq!(response.chunk().await.unwrap().unwrap(), "first");
                    assert!(response.bytes().await.is_err(), "truncated response must not appear complete");
                } else {
                    assert_eq!(response.status(), 504);
                    assert_eq!(response.text().await.unwrap(), "Upstream response timed out");
                }
            });
        }
        wait_count(&origin.state, path, 32).await;
        let healthy = client.get(origin.url("/review/healthy-after-timeout")).send();
        tokio::pin!(healthy);
        assert!(tokio::time::timeout(Duration::from_millis(200), &mut healthy).await.is_err());
        assert_eq!(origin.state.count("/review/healthy-after-timeout"), 0);
        let response = tokio::time::timeout(Duration::from_secs(35), healthy).await.unwrap().unwrap();
        assert_eq!(response.status(), 200);
        response.bytes().await.unwrap();
        while let Some(result) = stalled.join_next().await { result.unwrap(); }
        assert_eq!(origin.state.count(path), 32, "no automatic replay");
        assert_eq!(origin.state.count("/review/healthy-after-timeout"), 1);
        assert_eq!(cache.counters.failures.load(Ordering::Relaxed), 32, "one failure per timeout");
        gateway.close().await;
        origin.close().await;
    }
    tokio::join!(check(false, false), check(true, false), check(false, true), check(true, true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forwarding_idle_timeout_preserves_long_uploads_responses_and_backpressure() {
    async fn check(h2: bool) {
        let origin = Origin::with_h2(true, h2).await;
        let net = network(&origin);
        let make = |path: &str, body| http::Request::builder().uri(format!("https://{}{path}", origin.address))
            .header("te", "trailers").body(body).unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(2);
        let upload = tokio::spawn(async move {
            for part in [b"original".as_slice(), b"\x00\xff", b"body"] {
                if part != b"original" { tokio::time::sleep(Duration::from_secs(20)).await; }
                tx.send(Ok(hyper::body::Frame::data(Bytes::copy_from_slice(part)))).await.unwrap();
            }
        });
        let target = "/review/a/../b?x=%2f&x=%2F&empty=";
        let mut request = make(target, FrameBody(rx).boxed_unsync());
        *request.method_mut() = Method::PATCH;
        let active_upload = async {
            let response = net.forward(request).await.unwrap();
            assert_eq!(response.status(), 200);
            let echo: serde_json::Value = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
            assert_eq!(echo["target"], target);
            assert_eq!(echo["method"], "PATCH");
            assert_eq!(echo["digest"], gbf_flash_cache_core::hash(b"original\x00\xffbody"));
            assert_eq!(echo["length"], 14);
        };
        let active_response = async {
            let body = net.forward(make("/slow-stream", test_body(Bytes::new()))).await.unwrap()
                .into_body().collect().await.unwrap();
            assert_eq!(body.trailers().unwrap()["x-checksum"], "synthetic-checksum");
            assert_eq!(body.to_bytes(), "firstsecondthird");
        };
        let slow_reader = async {
            let mut body = net.forward(make("/stream", test_body(Bytes::new()))).await.unwrap().into_body();
            assert_eq!(body.frame().await.unwrap().unwrap().into_data().unwrap(), "first");
            // No body polls while the consumer is blocked: do not call this upstream idleness.
            tokio::time::sleep(Duration::from_secs(35)).await;
            origin.state.gate.notify_one();
            let rest = body.collect().await.unwrap();
            assert_eq!(rest.trailers().unwrap()["x-checksum"], "synthetic-checksum");
            assert_eq!(rest.to_bytes(), "second");
        };
        let stalled_upload = async {
            let (_keep_open, rx) = tokio::sync::mpsc::channel(1);
            let mut req = make("/review/stalled-upload", FrameBody(rx).boxed_unsync());
            *req.method_mut() = Method::POST;
            assert_eq!(net.forward(req).await.unwrap_err(), "upstream_forward_timeout");
        };
        tokio::time::timeout(Duration::from_secs(48), async {
            tokio::join!(active_upload, active_response, slow_reader, stalled_upload);
            upload.await.unwrap();
        }).await.unwrap();
        assert_eq!(origin.state.count("/review/a/../b"), 1);
        assert_eq!(origin.state.count("/review/stalled-upload"), 1);
        origin.close().await;
    }
    tokio::join!(check(false), check(true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forwarding_cancel_releases_slots_and_stop_cancels_waiters() {
    let origin = Origin::new(true).await;
    let home = tempfile::tempdir().unwrap();
    let cache = Cache::new(home.path().join("cache"), 0, 3600, network(&origin)).unwrap();
    let make = |path: &str| http::Request::builder().uri(origin.url(path).as_str()).body(test_body(Bytes::new())).unwrap();
    let mut blocked = tokio::task::JoinSet::new();
    for _ in 0..32 {
        let cache = cache.clone(); let req = make("/wait-headers");
        blocked.spawn(async move { cache.forward(req).await });
    }
    wait_count(&origin.state, "/wait-headers", 32).await;
    blocked.abort_all();
    while blocked.join_next().await.is_some() {}
    let (reply, permit) = tokio::time::timeout(Duration::from_secs(3), cache.forward(make("/review/after-cancel"))).await.unwrap().unwrap();
    assert_eq!(reply.status(), 200); reply.into_body().collect().await.unwrap(); drop(permit);
    // Hold all returned permits to prove stop wakes a waiter even before permit acquisition.
    let mut permits = Vec::new();
    for _ in 0..32 {
        let (reply, permit) = cache.forward(make("/review/hold")).await.unwrap();
        reply.into_body().collect().await.unwrap(); permits.push(permit);
    }
    let waiting = cache.forward(make("/review/stopped-waiter"));
    tokio::pin!(waiting);
    assert!(tokio::time::timeout(Duration::from_millis(50), &mut waiting).await.is_err());
    cache.cancel.cancel();
    assert_eq!(tokio::time::timeout(Duration::from_secs(1), waiting).await.unwrap().unwrap_err(), "stopped");
    assert_eq!(origin.state.count("/review/stopped-waiter"), 0);
    drop(permits);
    cache.close().await; origin.close().await;
}

// Freeze an established TCP path without closing it;
// new connections still reach the origin normally.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn http2_blackhole_is_retired_without_replaying_requests() {
    async fn check(asset: bool, idle: bool) {
        let origin = Origin::with_h2(true, true).await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let frozen = CancellationToken::new();
        let freeze = frozen.clone();
        let connections = Arc::new(AtomicU64::new(0));
        let count = connections.clone();
        let destination = origin.address;
        let relay = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                let (mut client, _) = listener.accept().await.unwrap();
                let first = count.fetch_add(1, Ordering::SeqCst) == 0;
                let freeze = freeze.clone();
                tasks.spawn(async move {
                    let mut server = TcpStream::connect(destination).await.unwrap();
                    if first {
                        tokio::select! {
                            _ = freeze.cancelled() => {},
                            _ = tokio::io::copy_bidirectional(&mut client, &mut server) => return,
                        }
                        // Hold both sockets open while discarding further forwarding.
                        std::future::pending::<()>().await;
                        drop((client, server));
                    } else {
                        let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
                    }
                });
            }
        });
        let net = Network::new(None, std::slice::from_ref(&origin.root)).unwrap();
        let make = || http::Request::builder().uri(format!("https://{address}/review/node-switch"))
            .body(test_body(Bytes::new())).unwrap();
        let send = || async {
            if asset {
                let reply = net.fetch(&Request {
                    method: Method::GET, url: make().uri().clone(),
                    headers: HeaderMap::new(), body: Bytes::new(),
                }, true).await?;
                assert_eq!(reply.protocol, http::Version::HTTP_2);
            } else {
                let reply = net.forward(make()).await?;
                assert_eq!(reply.version(), http::Version::HTTP_2);
                reply.into_body().collect().await.map_err(|e| e.to_string())?;
            }
            Ok::<_, String>(())
        };
        send().await.unwrap();
        frozen.cancel();
        tokio::time::sleep(Duration::from_millis(50)).await;
        if idle {
            // No request is needed to detect and retire the dead connection.
            tokio::time::sleep(Duration::from_secs(28)).await;
        } else {
            let start = std::time::Instant::now();
            let error = tokio::time::timeout(Duration::from_secs(29), send()).await
                .expect("HTTP/2 heartbeat must fail before the request deadline").unwrap_err();
            assert_ne!(error, "upstream_forward_timeout");
            assert_eq!(connections.load(Ordering::SeqCst), 1, "failed request must not be replayed");
            assert_eq!(origin.state.count("/review/node-switch"), 1);
            println!("asset={asset}: dead connection rejected in {:.1}s without replay", start.elapsed().as_secs_f64());
        }
        tokio::time::timeout(Duration::from_secs(3), send()).await
            .expect("next request must use a fresh connection").unwrap();
        assert_eq!(connections.load(Ordering::SeqCst), 2);
        assert_eq!(origin.state.count("/review/node-switch"), 2);
        println!("asset={asset}, idle={idle}: next request recovered on a new connection");
        relay.abort();
        let _ = relay.await;
        origin.close().await;
    }
    tokio::join!(check(false, false), check(true, false), check(false, true));
}
