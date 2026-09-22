use crate::{Headers, BODY_LIMIT};
use bytes::Bytes;
use http::{HeaderMap, Method, StatusCode};
use std::{collections::HashSet, future::Future, pin::Pin, sync::{Arc, Mutex}, task::{Context, Poll}, time::Duration};
use http_body_util::BodyExt;
use hyper::body::Body as _;

pub type Body = http_body_util::combinators::UnsyncBoxBody<Bytes, Box<dyn std::error::Error + Send + Sync>>;

pub type Result<T> = std::result::Result<T, String>;

const FORWARD_IDLE: Duration = Duration::from_secs(30);

// Upload progress refreshes the header wait; a long, active upload has no total deadline.
struct UploadBody {
    inner: Body,
    progress: Option<tokio::sync::watch::Sender<tokio::time::Instant>>,
}
impl hyper::body::Body for UploadBody {
    type Data = Bytes;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>)
        -> Poll<Option<std::result::Result<hyper::body::Frame<Bytes>, Self::Error>>> {
        let frame = std::task::ready!(Pin::new(&mut self.inner).poll_frame(cx));
        let finished = frame.is_none() || self.inner.is_end_stream();
        let progressed = frame.as_ref().is_some_and(|r| r.as_ref().is_ok_and(|f|
            f.data_ref().is_some_and(|b| !b.is_empty()) || f.trailers_ref().is_some()));
        if finished || progressed {
            if let Some(progress) = &self.progress { progress.send_replace(tokio::time::Instant::now()); }
        }
        if finished { self.progress.take(); }
        Poll::Ready(frame)
    }
    fn is_end_stream(&self) -> bool { self.inner.is_end_stream() }
    fn size_hint(&self) -> hyper::body::SizeHint { self.inner.size_hint() }
}

struct IdleBody {
    inner: Option<Body>,
    idle: Option<Pin<Box<tokio::time::Sleep>>>,
}
impl hyper::body::Body for IdleBody {
    type Data = Bytes;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>)
        -> Poll<Option<std::result::Result<hyper::body::Frame<Bytes>, Self::Error>>> {
        let Some(inner) = self.inner.as_mut() else { return Poll::Ready(None); };
        match Pin::new(inner).poll_frame(cx) {
            Poll::Ready(frame) => {
                self.idle = None;
                if frame.is_none() || frame.as_ref().is_some_and(|r| r.is_err()) { self.inner.take(); }
                Poll::Ready(frame)
            }
            Poll::Pending => {
                // ponytail: time only a pending read; downstream backpressure is not upstream idleness.
                let idle = self.idle.get_or_insert_with(|| Box::pin(tokio::time::sleep(FORWARD_IDLE)));
                if idle.as_mut().poll(cx).is_pending() { return Poll::Pending; }
                self.inner.take();
                self.idle = None;
                Poll::Ready(Some(Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "upstream_body_timeout").into())))
            }
        }
    }
    fn is_end_stream(&self) -> bool { self.inner.as_ref().is_none_or(|body| body.is_end_stream()) }
    fn size_hint(&self) -> hyper::body::SizeHint { self.inner.as_ref().map_or_else(hyper::body::SizeHint::default, |body| body.size_hint()) }
}

#[derive(Clone)]
pub struct Request {
    pub method: Method,
    pub url: http::Uri,
    pub headers: HeaderMap,
    pub body: Bytes,
}
#[derive(Clone, Debug)]
pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
    pub protocol: http::Version,
    pub stream: Option<Arc<Streaming>>,
}

// Single-flight shares buffered replies. An oversized response has exactly one reader.
pub struct Streaming(pub Mutex<Option<Body>>);
impl std::fmt::Debug for Streaming {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("Streaming") }
}
struct OverflowBody {
    prefix: std::array::IntoIter<Bytes, 2>,
    inner: hyper::body::Incoming,
    idle: Pin<Box<tokio::time::Sleep>>,
    deadline: tokio::time::Instant,
}
impl hyper::body::Body for OverflowBody {
    type Data = Bytes;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>)
        -> Poll<Option<std::result::Result<hyper::body::Frame<Bytes>, Self::Error>>> {
        if self.idle.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Some(Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout").into())));
        }
        if let Some(prefix) = self.prefix.next() {
            return Poll::Ready(Some(Ok(hyper::body::Frame::data(prefix))));
        }
        let frame = std::task::ready!(Pin::new(&mut self.inner).poll_frame(cx));
        let next = self.deadline.min(tokio::time::Instant::now() + Duration::from_secs(30));
        self.idle.as_mut().reset(next);
        Poll::Ready(frame.map(|frame| frame.map_err(|error| Box::new(error) as Self::Error)))
    }
}

pub fn clean_headers(headers: &HeaderMap, keep_length: bool) -> HeaderMap {
    let mut remove: HashSet<String> = [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "host",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    if !keep_length {
        remove.insert("content-length".into());
    }
    for value in headers.get_all("connection") {
        if let Ok(value) = value.to_str() {
            remove.extend(value.split(',').map(|s| s.trim().to_ascii_lowercase()));
        }
    }
    let mut result = headers.clone();
    for name in remove {
        result.remove(&name);
    }
    result
}
/// Preserve end-to-end fields; only framing and hop-by-hop fields belong to the gateway.
pub fn forwarding_headers(headers: &HeaderMap) -> HeaderMap {
    let mut result = clean_headers(headers, true);
    for name in ["host", "trailer", "te"] {
        let hop = headers.get_all("connection").iter().filter_map(|v| v.to_str().ok())
            .any(|v| v.split(',').any(|v| v.trim().eq_ignore_ascii_case(name)));
        if !hop {
            for value in headers.get_all(name) {
                if name != "te" || value.as_bytes().eq_ignore_ascii_case(b"trailers") {
                    // Hyper's HTTP/2 filter only accepts the lowercase spelling.
                    result.append(name, if name == "te" {
                        http::HeaderValue::from_static("trailers")
                    } else { value.clone() });
                }
            }
        }
    }
    result
}
pub fn text_headers(headers: &HeaderMap) -> Option<Headers> {
    headers
        .iter()
        .map(|(n, v)| Some((n.to_string(), v.to_str().ok()?.to_owned())))
        .collect()
}
pub fn http_headers(headers: &Headers) -> Result<HeaderMap> {
    let mut result = HeaderMap::new();
    for (n, v) in headers {
        result.append(
            http::HeaderName::from_bytes(n.as_bytes()).map_err(|_| "cache header")?,
            http::HeaderValue::from_str(v).map_err(|_| "cache header")?,
        );
    }
    Ok(result)
}
/// API and assets have independent pools. Original bodies and headers are forwarded without decoding.
pub struct Network {
    assets: Client,
    api: Client,
    pub trace: Option<std::sync::Arc<crate::trace::Trace>>,
}
type Client = hyper_util::client::legacy::Client<Connector, Body>;
impl Network {
    pub fn new(upstream: Option<&str>, extra_roots: &[Vec<u8>]) -> Result<Self> {
        let connector = Connector(std::sync::Arc::new(crate::tunnel::Route::new(
            upstream,
            extra_roots,
        )?));
        let build = || {
            hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                .pool_timer(hyper_util::rt::TokioTimer::new())
                .pool_idle_timeout(Duration::from_secs(60))
                .pool_max_idle_per_host(16)
                // Retire unresponsive HTTP/2 connections, including idle pooled ones.
                .timer(hyper_util::rt::TokioTimer::new())
                .http2_keep_alive_interval(Duration::from_secs(15))
                .http2_keep_alive_timeout(Duration::from_secs(10))
                .http2_keep_alive_while_idle(true)
                .retry_canceled_requests(false)
                .build(connector.clone())
        };
        Ok(Self {
            assets: build(),
            api: build(),
            trace: None,
        })
    }
    pub fn with_trace(mut self, trace: std::sync::Arc<crate::trace::Trace>) -> Self {
        self.trace = Some(trace);
        self
    }
    pub async fn forward(&self, request: http::Request<Body>) -> Result<http::Response<Body>> {
        let (progress, mut updates) = tokio::sync::watch::channel(tokio::time::Instant::now());
        let request = request.map(|inner| {
            let progress = if inner.is_end_stream() { None } else { Some(progress) };
            UploadBody { inner, progress }.boxed_unsync()
        });
        let response = self.api.request(request);
        tokio::pin!(response);
        let mut uploading = true;
        loop {
            let deadline = *updates.borrow_and_update() + FORWARD_IDLE;
            tokio::select! {
                biased;
                result = &mut response => return result.map(|response| response.map(|body| IdleBody {
                    inner: Some(body.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>).boxed_unsync()),
                    idle: None,
                }.boxed_unsync())).map_err(upstream_error),
                changed = updates.changed(), if uploading => { uploading = changed.is_ok(); }
                _ = tokio::time::sleep_until(deadline) => return Err("upstream_forward_timeout".into()),
            }
        }
    }
    pub async fn fetch(&self, request: &Request, asset: bool) -> Result<Reply> {
        let start = std::time::Instant::now();
        if let Some(log) = &self.trace {
            log.event(
                "NETWORK_START",
                Some(&request.url),
                serde_json::json!({"method":request.method.as_str(),"asset":asset}),
            );
        }
        let result = tokio::time::timeout(Duration::from_secs(40), self.download(request, asset))
            .await
            .unwrap_or_else(|_| Err("timeout".into()));
        if let Some(log) = &self.trace {
            match &result {Ok(reply)=>log.event(if reply.stream.is_some() { "NETWORK_STREAM" } else { "NETWORK_DONE" },Some(&request.url),serde_json::json!({"status":reply.status.as_u16(),"bytes":reply.body.len(),"protocol":format!("{:?}",reply.protocol),"elapsed_ms":start.elapsed().as_secs_f64()*1000.})),Err(error)=>log.event("NETWORK_FAILED",Some(&request.url),serde_json::json!({"cause":error,"elapsed_ms":start.elapsed().as_secs_f64()*1000.}))}
        }
        result
    }
    async fn download(&self, request: &Request, asset: bool) -> Result<Reply> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
        let mut outgoing = http::Request::builder()
            .method(request.method.clone())
            .uri(request.url.clone())
            .body(http_body_util::Full::new(request.body.clone()).map_err(|never| match never {}).boxed_unsync())
            .map_err(|_| "invalid request")?;
        *outgoing.headers_mut() = clean_headers(&request.headers, false);
        let response = if asset { &self.assets } else { &self.api }
            .request(outgoing)
            .await
            .map_err(upstream_error)?;
        let protocol = response.version();
        let status = response.status();
        let headers = forwarding_headers(response.headers());
        let mut stream = response.into_body();
        let mut body = Vec::new();
        while let Some(frame) = tokio::time::timeout(Duration::from_secs(30), stream.frame())
            .await
            .map_err(|_| "timeout")?
        {
            let frame = frame.map_err(|_| "response_body")?;
            if let Ok(chunk) = frame.into_data() {
                if body.len() + chunk.len() > BODY_LIMIT {
                    if !asset { return Err("response_size_limit".into()); }
                    let overflow = OverflowBody {
                        prefix: [body.into(), chunk].into_iter(), inner: stream, deadline,
                        idle: Box::pin(tokio::time::sleep_until(deadline.min(tokio::time::Instant::now() + Duration::from_secs(30)))),
                    };
                    return Ok(Reply { status, headers, protocol, body: Bytes::new(),
                        stream: Some(Arc::new(Streaming(Mutex::new(Some(overflow.boxed_unsync()))))) });
                }
                body.extend_from_slice(&chunk);
            }
        }
        Ok(Reply {
            status,
            headers: clean_headers(&headers, request.method == Method::HEAD),
            body: body.into(),
            protocol,
            stream: None,
        })
    }
}
// Only emit classified errors: underlying messages can contain URLs or credentials.
fn upstream_error(error: hyper_util::client::legacy::Error) -> String {
    use std::error::Error;
    let mut source = error.source();
    while let Some(cause) = source {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            let stage = io.to_string();
            if ["connect", "connect_timeout", "proxy_connect", "proxy_handshake",
                "proxy_upgrade", "proxy_rejected", "proxy_request", "socket",
                "tls", "tls_timeout", "tls_certificate", "tls_protocol", "tls_eof"]
                .contains(&stage.as_str()) {
                return format!("upstream_{stage}");
            }
            if io.kind() != std::io::ErrorKind::Other {
                return format!("upstream_io_{:?}", io.kind()).to_ascii_lowercase();
            }
        }
        if let Some(http) = cause.downcast_ref::<hyper::Error>() {
            if http.is_incomplete_message() { return "upstream_incomplete_response".into(); }
            if http.is_canceled() { return "upstream_canceled".into(); }
            if http.is_closed() { return "upstream_closed".into(); }
            if http.is_parse() { return "upstream_http_parse".into(); }
        }
        source = cause.source();
    }
    if error.is_connect() { "upstream_connect" } else { "upstream_io" }.into()
}

#[derive(Clone)]
struct Connector(std::sync::Arc<crate::tunnel::Route>);
impl tower_service::Service<http::Uri> for Connector {
    type Response = Connected;
    type Error = std::io::Error;
    type Future =
        std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<Connected>> + Send>>;
    fn poll_ready(
        &mut self,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn call(&mut self, uri: http::Uri) -> Self::Future {
        let route = self.0.clone();
        Box::pin(async move {
            let host = uri
                .host()
                .ok_or_else(|| std::io::Error::other("invalid host"))?
                .trim_matches(['[', ']']);
            let https = uri.scheme_str() == Some("https");
            let port = uri.port_u16().unwrap_or(if https { 443 } else { 80 });
            let stream = route
                .open(host, port)
                .await
                .map_err(std::io::Error::other)?;
            let (stream, h2): (crate::tunnel::BoxStream, bool) = if https {
                let stream = route
                    .secure_protocol(stream, host, true)
                    .await
                    .map_err(std::io::Error::other)?;
                let h2 = stream.get_ref().1.alpn_protocol() == Some(b"h2");
                (Box::new(stream), h2)
            } else {
                (stream, false)
            };
            Ok(Connected {
                io: hyper_util::rt::TokioIo::new(stream),
                h2,
            })
        })
    }
}
struct Connected {
    io: hyper_util::rt::TokioIo<crate::tunnel::BoxStream>,
    h2: bool,
}
impl hyper_util::client::legacy::connect::Connection for Connected {
    fn connected(&self) -> hyper_util::client::legacy::connect::Connected {
        let info = hyper_util::client::legacy::connect::Connected::new();
        if self.h2 {
            info.negotiated_h2()
        } else {
            info
        }
    }
}
impl hyper::rt::Read for Connected {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: hyper::rt::ReadBufCursor<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.io).poll_read(cx, buf)
    }
}
impl hyper::rt::Write for Connected {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.io).poll_write(cx, buf)
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.io).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.io).poll_shutdown(cx)
    }
}
