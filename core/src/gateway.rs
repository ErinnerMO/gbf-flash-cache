use crate::{
    certificates::Authority,
    engine::Engine,
    network::{clean_headers, forwarding_headers, Reply, Request, Result},
    tunnel::{authority, BoxStream, Route},
    BODY_LIMIT,
};
use bytes::Bytes;
use http::{Method, StatusCode};
use http_body_util::{combinators::UnsyncBoxBody, BodyExt, Full};
use hyper::{body::{Body as _, Incoming}, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::{
    collections::HashMap,
    convert::Infallible,
    error::Error,
    future::Future,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
};
use tokio_rustls::{rustls, TlsAcceptor};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use url::Url;
type Body = UnsyncBoxBody<Bytes, Box<dyn Error + Send + Sync>>;
fn full(bytes: impl Into<Bytes>) -> Body {
    Full::new(bytes.into())
        .map_err(|never| match never {})
        .boxed_unsync()
}
fn error(status: StatusCode, message: &str) -> http::Response<Body> {
    http::Response::builder()
        .status(status)
        .header("connection", "close")
        .body(full(message.to_owned()))
        .unwrap()
}

pub struct Gateway {
    pub address: SocketAddr,
    engine: Arc<Engine>,
    route: Route,
    origins: HashMap<String, (Url, TlsAcceptor)>,
    authority: Authority,
    outer: std::sync::Mutex<HashMap<IpAddr, TlsAcceptor>>,
    cancel: CancellationToken,
    tasks: TaskTracker,
    slots: Arc<Semaphore>,
}
impl Gateway {
    pub async fn start(
        port: u16,
        engine: Arc<Engine>,
        ca: &Authority,
        origins: &[Url],
        route: Route,
    ) -> Result<Arc<Self>> {
        Self::start_with_lan(port, engine, ca, origins, route, false).await
    }
    pub async fn start_with_lan(
        port: u16,
        engine: Arc<Engine>,
        ca: &Authority,
        origins: &[Url],
        route: Route,
        lan: bool,
    ) -> Result<Arc<Self>> {
        let mut targets = HashMap::new();
        for origin in origins {
            if origin.scheme() != "https"
                || origin.host_str().is_none()
                || !origin.username().is_empty()
                || origin.password().is_some()
                || origin.path() != "/"
                || origin.query().is_some()
                || origin.fragment().is_some()
            {
                return Err("invalid inspected origin".into());
            }
            let host = origin.host_str().unwrap();
            targets.insert(
                authority(host, origin.port_or_known_default().unwrap()),
                (origin.clone(), tls(ca, host)?),
            );
        }
        let listener = TcpListener::bind((
            if lan {
                std::net::Ipv4Addr::UNSPECIFIED
            } else {
                std::net::Ipv4Addr::LOCALHOST
            },
            port,
        ))
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                format!("端口 {port} 已被占用")
            } else {
                format!("无法监听端口 {port}：{e}")
            }
        })?;
        let gateway = Arc::new(Self {
            address: listener.local_addr().map_err(|_| "listen_failed")?,
            engine,
            route,
            origins: targets,
            authority: ca.clone(),
            outer: std::sync::Mutex::new(HashMap::new()),
            cancel: CancellationToken::new(),
            tasks: TaskTracker::new(),
            slots: Arc::new(Semaphore::new(256)),
        });
        let g = gateway.clone();
        gateway.spawn(async move {
            while let Ok((stream, peer)) = listener.accept().await {
                if !local_peer(peer.ip()) {
                    continue;
                }
                let Ok(permit) = g.slots.clone().try_acquire_owned() else {
                    drop(stream);
                    continue;
                };
                let g2 = g.clone();
                g.spawn(async move {
                    let _permit = permit;
                    let _ = g2.accept(stream).await;
                });
            }
        });
        Ok(gateway)
    }
    fn outer_tls(&self, ip: IpAddr) -> Result<TlsAcceptor> {
        // Only actual local socket addresses can select a certificate, never client SNI.
        let mut acceptors = self.outer.lock().map_err(|_| "TLS setup")?;
        if let Some(acceptor) = acceptors.get(&ip) {
            return Ok(acceptor.clone());
        }
        let acceptor = tls(&self.authority, &ip.to_string())?;
        acceptors.insert(ip, acceptor.clone());
        Ok(acceptor)
    }
    fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) {
        let cancel = self.cancel.clone();
        self.tasks.spawn(async move {
            tokio::select! {_=cancel.cancelled()=>{},_=future=>{}}
        });
    }
    async fn accept(self: Arc<Self>, mut stream: TcpStream) -> Result<()> {
        stream.set_nodelay(true).map_err(|_| "socket")?;
        let mut first = [0];
        tokio::time::timeout(Duration::from_secs(10), stream.peek(&mut first))
            .await
            .map_err(|_| "handshake_timeout")?
            .map_err(|_| "handshake")?;
        if matches!(first[0], 4 | 5) {
            let (command, host, port) = tokio::time::timeout(Duration::from_secs(10), socks(&mut stream))
                .await
                .map_err(|_| "handshake_timeout")??;
            if command == 3 {
                return crate::udp::associate(stream, &host, port, self.route.udp_route()?).await.map_err(|e| e.to_string());
            }
            let host = host.to_ascii_lowercase();
            self.ingress(if first[0] == 4 { "SOCKS4" } else { "SOCKS5" }, &host, port, "accepted");
            self.check_target(&host, port).await?;
            let remote = if self.origins.contains_key(&authority(&host, port)) || self.inspect_ip(&host, port) {
                None
            } else {
                match self.route.open(&host, port).await {
                    Ok(remote) => Some(remote),
                    Err(e) => {
                        let reply: &[u8] = if first[0] == 4 {
                            &[0, 91, 0, 0, 0, 0, 0, 0]
                        } else {
                            &[5, 1, 0, 1, 0, 0, 0, 0, 0, 0]
                        };
                        let _ = stream.write_all(reply).await;
                        return Err(e);
                    }
                }
            };
            stream
                .write_all(if first[0] == 4 {
                    &[0, 90, 0, 0, 0, 0, 0, 0]
                } else {
                    &[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]
                })
                .await
                .map_err(|_| "handshake")?;
            return self.tunnel(Box::new(stream), host, port, remote).await;
        }
        let stream: BoxStream = if first[0] == 22 {
            Box::new(
                tokio::time::timeout(
                    Duration::from_secs(10),
                    self.outer_tls(stream.local_addr().map_err(|_| "socket")?.ip())?
                        .accept(stream),
                )
                .await
                .map_err(|_| "tls_timeout")?
                .map_err(|_| "tls")?,
            )
        } else {
            Box::new(stream)
        };
        self.http(stream, None).await;
        Ok(())
    }
    async fn check_target(&self, host: &str, port: u16) -> Result<()> {
        let host = crate::tunnel::unbracket(host);
        if port == 0
            || host.is_empty()
            || host.len() > 253
            || !host
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-_:".contains(&c))
        {
            return Err("invalid target".into());
        }
        crate::tunnel::check_loop(host, port, self.address).await?;
        Ok(())
    }
    fn ingress(&self, protocol: &str, host: &str, port: u16, action: &str) {
        if let Some(log) = &self.engine.cache.trace {
            log.event("INGRESS", None, serde_json::json!({"protocol":protocol,"host":host,"port":port,"action":action}));
        }
    }
    fn inspect_ip(&self, host: &str, port: u16) -> bool {
        host.parse::<IpAddr>().is_ok()
            && self.origins.values().any(|(url, _)| url.port_or_known_default() == Some(port))
    }
    async fn tunnel(
        self: Arc<Self>,
        mut stream: BoxStream,
        mut host: String,
        port: u16,
        remote: Option<BoxStream>,
    ) -> Result<()> {
        if self.inspect_ip(&host, port) && !self.origins.contains_key(&authority(&host, port)) {
            let (name, replay) = client_hello(stream).await?;
            stream = replay;
            self.ingress("TLS", name.as_deref().unwrap_or(&host), port, if name.is_some() { "sni" } else { "no_sni" });
            if let Some(name) = name.filter(|name| self.origins.contains_key(&authority(name, port))) {
                host = name;
            }
        }
        if let Some((origin, tls)) = self.origins.get(&authority(&host, port)) {
            self.ingress("TLS", &host, port, "inspect");
            let stream = match tokio::time::timeout(Duration::from_secs(10), tls.accept(stream)).await {
                Ok(Ok(stream)) => stream,
                Ok(Err(_)) => { self.ingress("TLS", &host, port, "handshake_failed"); return Err("tls".into()); }
                Err(_) => { self.ingress("TLS", &host, port, "handshake_timeout"); return Err("tls_timeout".into()); }
            };
            let origin = origin.clone();
            self.http(Box::new(stream), Some(origin)).await;
        } else {
            self.ingress("TCP", &host, port, "relay");
            let mut remote = if let Some(remote) = remote {
                remote
            } else {
                self.route.open(&host, port).await?
            };
            tokio::io::copy_bidirectional(&mut stream, &mut remote)
                .await
                .map_err(|_| "relay_io")?;
        }
        Ok(())
    }
    fn http(
        self: Arc<Self>,
        stream: BoxStream,
        origin: Option<Url>,
    ) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            let service = service_fn(move |request| {
                let g = self.clone();
                let origin = origin.clone();
                async move {
                    Ok::<_, Infallible>(match g.handle(request, origin).await {
                        Ok(reply) => reply,
                        Err(e) => error(
                            StatusCode::BAD_GATEWAY,
                            &format!("GBF Flash Cache gateway failure: {e}"),
                        ),
                    })
                }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(Duration::from_secs(30))
                .max_buf_size(65536)
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades()
                .await;
        })
    }
    async fn handle(
        self: Arc<Self>,
        mut request: http::Request<Incoming>,
        origin: Option<Url>,
    ) -> Result<http::Response<Body>> {
        // Hyper removes chunked framing only; other transfer codings cannot be forwarded to H2.
        if request.headers().get_all("transfer-encoding").iter().any(|value| {
            value.to_str().map_or(true, |value| value.split(',')
                .any(|coding| !coding.trim().eq_ignore_ascii_case("chunked")))
        }) {
            return Ok(error(StatusCode::NOT_IMPLEMENTED, "Unsupported Transfer-Encoding"));
        }
        if request.method() == Method::CONNECT {
            if origin.is_some()
                || request.uri().scheme().is_some()
                || request.uri().path_and_query().is_some()
                || request.headers().contains_key("transfer-encoding")
                || request
                    .headers()
                    .get("content-length")
                    .is_some_and(|v| v != "0")
            {
                return Ok(error(StatusCode::BAD_REQUEST, "Invalid CONNECT"));
            }
            let Some(target) = request.uri().authority() else {
                return Ok(error(StatusCode::BAD_REQUEST, "Invalid CONNECT target"));
            };
            let host = target.host().trim_matches(['[', ']']).to_ascii_lowercase();
            let Some(port) = target.port_u16() else {
                return Ok(error(StatusCode::BAD_REQUEST, "Missing CONNECT port"));
            };
            let host = host.to_ascii_lowercase();
            self.ingress("CONNECT", &host, port, "accepted");
            self.check_target(&host, port).await?;
            let remote = if self.origins.contains_key(&authority(&host, port)) || self.inspect_ip(&host, port) {
                None
            } else {
                Some(self.route.open(&host, port).await?)
            };
            let upgraded = hyper::upgrade::on(&mut request);
            let g = self.clone();
            self.spawn(async move {
                if let Ok(stream) = upgraded.await {
                    let _ = g
                        .tunnel(Box::new(TokioIo::new(stream)), host, port, remote)
                        .await;
                }
            });
            return Ok(http::Response::new(full(Bytes::new())));
        }
        // Only attach the tunnel origin; never normalize a browser's path or query.
        let target = if let Some(origin) = &origin {
            let path = request.uri().path_and_query().ok_or("invalid path")?.clone();
            if request.uri().scheme().is_some() || request.uri().authority().is_some()
                || (!path.as_str().starts_with('/') && !(request.method() == Method::OPTIONS && path.as_str() == "*"))
                || path.as_str().contains('#') {
                return Ok(error(StatusCode::BAD_REQUEST, "Invalid target"));
            }
            http::Uri::builder().scheme(origin.scheme())
                .authority(&origin[url::Position::BeforeHost..url::Position::AfterPort])
                .path_and_query(path).build().map_err(|_| "invalid target")?
        } else {
            if !matches!(request.uri().scheme_str(), Some("http" | "https")) {
                return Ok(error(StatusCode::BAD_REQUEST, "Absolute HTTP URL required"));
            }
            request.uri().clone()
        };
        if target.authority().is_some_and(|a| a.as_str().contains('@')) {
            return Ok(error(StatusCode::BAD_REQUEST, "Invalid target"));
        }
        self.check_target(target.host().ok_or("invalid host")?,
            target.port_u16().unwrap_or(if target.scheme_str() == Some("https") { 443 } else { 80 })).await?;
        let different_host = request.headers().get("host").is_some_and(|host| {
            target.authority().is_some_and(|authority| {
                !host.as_bytes().eq_ignore_ascii_case(authority.as_str().as_bytes())
            })
        });
        // ponytail: differing Host uses uncached HTTP/1.1; H2 needs a separate connection authority.
        if origin.is_none() || different_host || request.headers().contains_key("upgrade") {
            if let Some(log) = &self.engine.cache.trace {
                log.event("INGRESS_HTTP", Some(&target), serde_json::json!({"action":"relay","upgrade":request.headers().contains_key("upgrade")}));
            }
            return self.relay(request, target).await;
        }
        if !self.engine.is_asset(&target) || request.method() != Method::GET || !request.body().is_end_stream() {
            *request.uri_mut() = target.clone();
            return self.forward(request, target).await;
        }
        let (parts, _) = request.into_parts();
        let body = Bytes::new();
        let head = parts.method == Method::HEAD;
        let cached = self
            .engine
            .request(Request {
                method: parts.method,
                url: target,
                headers: clean_headers(&parts.headers, false),
                body,
            })
            .await?;
        let reply = cached.reply;
        let empty = head || matches!(reply.status.as_u16(), 204 | 205 | 304);
        if let Some(body) = cached.stream.filter(|_| !empty) {
            let mut response = http::Response::builder().status(reply.status).body(body).map_err(|_| "response")?;
            *response.headers_mut() = forwarding_headers(&reply.headers);
            return Ok(response);
        }
        let mut response = http::Response::builder()
            .status(reply.status)
            .body(full(if empty {
                Bytes::new()
            } else {
                reply.body.clone()
            }))
            .map_err(|_| "response")?;
        *response.headers_mut() = clean_headers(&reply.headers, head);
        if reply.status == StatusCode::RESET_CONTENT {
            response
                .headers_mut()
                .insert("content-length", http::HeaderValue::from_static("0"));
        }
        Ok(response)
    }
    async fn forward(self: Arc<Self>, mut request: http::Request<Incoming>, target: http::Uri) -> Result<http::Response<Body>> {
        let observed = Request { method: request.method().clone(), url: target,
            headers: request.headers().clone(), body: Bytes::new() };
        self.engine.observe_request(&observed);
        *request.headers_mut() = forwarding_headers(request.headers());
        let result = self.engine.cache.forward(request.map(|body| body
            .map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>).boxed_unsync())).await;
        let (mut response, permit) = match result {
            Ok(value) => value,
            Err(error) => {
                if let Some(log) = &self.engine.cache.trace {
                    log.event("NETWORK_FAILED", Some(&observed.url), serde_json::json!({"cause":error}));
                }
                self.engine.observe_response(observed, None);
                if error == "upstream_forward_timeout" {
                    return Ok(crate::gateway::error(StatusCode::GATEWAY_TIMEOUT, "Upstream response timed out"));
                }
                return Err(error);
            }
        };
        let reply = Reply { status: response.status(), headers: response.headers().clone(),
            body: Bytes::new(), protocol: response.version(), stream: None };
        *response.headers_mut() = forwarding_headers(response.headers());
        Ok(response.map(|inner| {
            let mut body = ObservedBody { inner, engine: self.engine.clone(),
                observation: Some((observed, reply)), bytes: Some(Vec::new()), permit: Some(permit) };
            if body.inner.is_end_stream() { body.complete(); }
            body.boxed_unsync()
        }))
    }
    async fn relay(
        self: Arc<Self>,
        mut request: http::Request<Incoming>,
        target: http::Uri,
    ) -> Result<http::Response<Body>> {
        let host = target.host().ok_or("invalid target")?;
        let port = target.port_u16().unwrap_or(if target.scheme_str() == Some("https") { 443 } else { 80 });
        let stream = self.route.open(host, port).await?;
        let stream: BoxStream = if target.scheme_str() == Some("https") {
            Box::new(self.route.secure(stream, host).await?)
        } else {
            stream
        };
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|_| "relay_handshake")?;
        self.spawn(async move {
            let _ = connection.with_upgrades().await;
        });
        let upgrade = request.headers().get("upgrade").cloned();
        let client_upgrade = upgrade.as_ref().map(|_| hyper::upgrade::on(&mut request));
        *request.uri_mut() = http::Uri::builder()
            .path_and_query(request.uri().path_and_query().ok_or("invalid path")?.clone())
            .build().map_err(|_| "invalid path")?;
        *request.headers_mut() = forwarding_headers(request.headers());
        if !request.headers().contains_key("host") {
            request.headers_mut().insert("host", target.authority().ok_or("invalid host")?
                .as_str().parse().map_err(|_| "invalid host")?);
        }
        if let Some(upgrade) = upgrade {
            request.headers_mut().insert("upgrade", upgrade);
            request
                .headers_mut()
                .insert("connection", http::HeaderValue::from_static("upgrade"));
        }
        let mut response = sender.send_request(request).await.map_err(|_| "relay_io")?;
        if response.status() == StatusCode::SWITCHING_PROTOCOLS {
            let client = client_upgrade.ok_or("unexpected upgrade")?;
            let server = hyper::upgrade::on(&mut response);
            self.spawn(async move {
                if let (Ok(client), Ok(server)) = tokio::join!(client, server) {
                    let _ = tokio::io::copy_bidirectional(
                        &mut TokioIo::new(client),
                        &mut TokioIo::new(server),
                    )
                    .await;
                }
            });
        } else {
            *response.headers_mut() = forwarding_headers(response.headers());
        }
        Ok(response.map(|body| {
            body.map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)
                .boxed_unsync()
        }))
    }
    pub async fn close(&self) {
        self.cancel.cancel();
        self.tasks.close();
        self.tasks.wait().await;
        self.engine.close().await;
    }
}
struct ObservedBody {
    inner: Body,
    engine: Arc<Engine>,
    observation: Option<(Request, Reply)>,
    bytes: Option<Vec<u8>>,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
}
impl ObservedBody {
    fn complete(&mut self) {
        self.permit.take();
        if let Some((request, mut reply)) = self.observation.take() {
            reply.body = self.bytes.take().unwrap_or_default().into();
            self.engine.observe_response(request, Some(Arc::new(reply)));
        }
    }
}
impl hyper::body::Body for ObservedBody {
    type Data = Bytes;
    type Error = Box<dyn Error + Send + Sync>;
    fn poll_frame(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>)
        -> std::task::Poll<Option<std::result::Result<hyper::body::Frame<Bytes>, Self::Error>>> {
        let frame = std::task::ready!(std::pin::Pin::new(&mut self.inner).poll_frame(cx));
        match frame {
            Some(Ok(frame)) => {
                if let (Some(data), Some(bytes)) = (frame.data_ref(), self.bytes.as_mut()) {
                    if bytes.len() + data.len() <= BODY_LIMIT { bytes.extend_from_slice(data); }
                    else { self.bytes = None; }
                }
                if self.inner.is_end_stream() { self.complete(); }
                std::task::Poll::Ready(Some(Ok(frame)))
            }
            Some(Err(error)) => {
                self.permit.take();
                if let Some((request, reply)) = self.observation.take() {
                    if !reply.status.is_client_error() && !reply.status.is_server_error() {
                        self.engine.cache.counters.failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    self.engine.observe_response(request, None);
                }
                std::task::Poll::Ready(Some(Err(error)))
            }
            None => { self.complete(); std::task::Poll::Ready(None) }
        }
    }
    fn size_hint(&self) -> hyper::body::SizeHint { self.inner.size_hint() }
}
async fn socks(stream: &mut TcpStream) -> Result<(u8, String, u16)> {
    let version = stream.read_u8().await.map_err(|_| "socks")?;
    if version == 4 {
        if stream.read_u8().await.map_err(|_| "socks")? != 1 {
            return Err("SOCKS CONNECT required".into());
        }
        let port = stream.read_u16().await.map_err(|_| "socks")?;
        let mut ip = [0; 4];
        stream.read_exact(&mut ip).await.map_err(|_| "socks")?;
        let _ = zero_string(stream).await?;
        let host = if ip[..3] == [0, 0, 0] && ip[3] != 0 {
            zero_string(stream).await?
        } else {
            std::net::Ipv4Addr::from(ip).to_string()
        };
        return Ok((1, host, port));
    }
    let length = stream.read_u8().await.map_err(|_| "socks")?;
    let mut methods = vec![0; length as usize];
    stream.read_exact(&mut methods).await.map_err(|_| "socks")?;
    if !methods.contains(&0) {
        let _ = stream.write_all(&[5, 255]).await;
        return Err("SOCKS authentication unsupported".into());
    }
    stream.write_all(&[5, 0]).await.map_err(|_| "socks")?;
    let mut command = [0; 3];
    stream.read_exact(&mut command).await.map_err(|_| "socks")?;
    if command[0] != 5 || command[2] != 0 || !matches!(command[1], 1 | 3) {
        stream.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await.map_err(|e| e.to_string())?;
        return Err("SOCKS command unsupported".into());
    }
    let host = match stream.read_u8().await.map_err(|_| "socks")? {
        1 => {
            let mut ip = [0; 4];
            stream.read_exact(&mut ip).await.map_err(|_| "socks")?;
            std::net::Ipv4Addr::from(ip).to_string()
        }
        4 => {
            let mut ip = [0; 16];
            stream.read_exact(&mut ip).await.map_err(|_| "socks")?;
            std::net::Ipv6Addr::from(ip).to_string()
        }
        3 => {
            let size = stream.read_u8().await.map_err(|_| "socks")?;
            let mut name = vec![0; size as usize];
            stream.read_exact(&mut name).await.map_err(|_| "socks")?;
            String::from_utf8(name).map_err(|_| "invalid SOCKS host")?
        }
        _ => return Err("invalid SOCKS address type".into()),
    };
    Ok((command[1], host, stream.read_u16().await.map_err(|_| "socks")?))
}
async fn zero_string(stream: &mut TcpStream) -> Result<String> {
    let mut bytes = vec![];
    for _ in 0..256 {
        let b = stream.read_u8().await.map_err(|_| "socks")?;
        if b == 0 {
            return String::from_utf8(bytes).map_err(|_| "invalid SOCKS string".into());
        }
        bytes.push(b);
    }
    Err("SOCKS string too long".into())
}

fn local_peer(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip
                    .to_ipv4_mapped()
                    .is_some_and(|ip| local_peer(IpAddr::V4(ip)))
        }
    }
}

// IP-address CONNECT/SOCKS (including Android TUN) still carries the website name in TLS.
// Read only ClientHello; replay every byte unchanged, including unrecognised/non-TLS traffic.
async fn client_hello(mut stream: BoxStream) -> Result<(Option<String>, BoxStream)> {
    let mut prefix = Vec::new();
    let mut acceptor = rustls::server::Acceptor::default();
    let name = tokio::time::timeout(Duration::from_secs(10), async {
        let mut buffer = [0u8; 4096];
        while prefix.len() < 65536 {
            let count = stream.read(&mut buffer).await?;
            if count == 0 { break; }
            prefix.extend_from_slice(&buffer[..count]);
            if prefix[0] != 22 { break; }
            let mut input = &buffer[..count];
            while !input.is_empty() {
                if acceptor.read_tls(&mut input)? == 0 { break; }
            }
            match acceptor.accept() {
                Ok(Some(hello)) => return Ok(hello.client_hello().server_name().map(str::to_ascii_lowercase)),
                Ok(None) => {},
                Err(_) => break,
            }
        }
        Ok::<_, std::io::Error>(None)
    }).await.unwrap_or(Ok(None)).map_err(|_| "client_hello_io")?;
    let (reader, writer) = tokio::io::split(stream);
    let replay = tokio::io::join(std::io::Cursor::new(prefix).chain(reader), writer);
    Ok((name, Box::new(replay)))
}

fn tls(ca: &Authority, host: &str) -> Result<TlsAcceptor> {
    let leaf = ca.leaf(host).map_err(|_| "certificate issuance")?;
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| "TLS setup")?
    .with_no_client_auth()
    .with_single_cert(
        vec![leaf.certificate.into(), ca.certificate.clone().into()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(leaf.key).into(),
    )
    .map_err(|_| "TLS certificate")?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(TlsAcceptor::from(Arc::new(config)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn sniffing_preserves_plain_and_invalid_tls_bytes() {
        for payload in [b"plain request".as_slice(), b"\x16\x03\x03\x00\x04bad!"] {
            let (mut client, server) = tokio::io::duplex(128);
            client.write_all(payload).await.unwrap();
            client.shutdown().await.unwrap();
            let (name, mut replay) = client_hello(Box::new(server)).await.unwrap();
            assert!(name.is_none());
            let mut received = Vec::new();
            replay.read_to_end(&mut received).await.unwrap();
            assert_eq!(received, payload);
            replay.write_all(b"response").await.unwrap();
            let mut response = [0; 8];
            client.read_exact(&mut response).await.unwrap();
            assert_eq!(&response, b"response");
        }
    }
}
