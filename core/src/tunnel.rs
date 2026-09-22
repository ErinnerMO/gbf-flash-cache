use crate::network::Result;
use base64::Engine as _;
use bytes::Bytes;
use http_body_util::Empty;
use hyper_util::rt::TokioIo;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
};
use tokio_rustls::{rustls, TlsConnector};
use url::Url;

/// Detect local aliases and interface addresses before routing back into the listener.
pub async fn check_loop(host: &str, port: u16, listen: std::net::SocketAddr) -> Result<()> {
    if port != listen.port() {
        return Ok(());
    }
    let host = unbracket(host);
    let addresses = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host, port)),
    )
    .await
    .map_err(|_| "地址解析超时")?
    .map_err(|_| "无法解析地址")?;
    for address in addresses {
        let ip = address.ip().to_canonical();
        let bound = listen.ip().to_canonical();
        if ip.is_ipv4() == bound.is_ipv4()
            && (ip == bound
                || ip.is_unspecified()
                || (bound.is_unspecified()
                    && (ip.is_loopback() || std::net::UdpSocket::bind((ip, 0)).is_ok())))
        {
            return Err("代理不能指向本应用（proxy_loop）".into());
        }
    }
    Ok(())
}

pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}
pub type BoxStream = Box<dyn Stream>;
pub struct Route {
    url: Option<Url>,
    tls: Arc<rustls::ClientConfig>,
}
impl Route {
    pub fn new(address: Option<&str>, extra_roots: &[Vec<u8>]) -> Result<Self> {
        let url = address
            .map(Url::parse)
            .transpose()
            .map_err(|_| "invalid proxy address")?;
        if let Some(u) = &url {
            if !["http", "https", "socks4", "socks4a", "socks5", "socks5h"].contains(&u.scheme())
                || u.host_str().is_none()
                || !matches!(u.path(), "" | "/")
                || u.query().is_some()
                || u.fragment().is_some()
            {
                return Err("invalid proxy address".into());
            }
            let user = decode(u.username())?;
            let password = decode(u.password().unwrap_or(""))?;
            validate_proxy_credentials(u.scheme(), &user, &password)?;
            if u.scheme().starts_with("socks4") && u.password().is_some() {
                return Err("SOCKS4 不支持密码".into());
            }
        }
        let mut roots = rustls::RootCertStore::empty();
        for root in extra_roots {
            roots
                .add(root.clone().into())
                .map_err(|_| "invalid root certificate")?;
        }
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| "TLS setup")?
        .with_root_certificates(roots)
        .with_no_client_auth();
        Ok(Self {
            url,
            tls: Arc::new(tls),
        })
    }
    pub fn udp_route(&self) -> Result<crate::udp::Route> {
        match &self.url {
            Some(u) if matches!(u.scheme(), "socks5" | "socks5h") => Ok(crate::udp::Route::Socks {
                host: unbracket(u.host_str().ok_or("invalid proxy host")?).to_owned(),
                port: u.port().unwrap_or(1080),
                user: decode(u.username())?,
                password: decode(u.password().unwrap_or(""))?,
            }),
            _ => Ok(crate::udp::Route::Direct),
        }
    }
    pub async fn open(&self, host: &str, port: u16) -> Result<BoxStream> {
        tokio::time::timeout(Duration::from_secs(10), self.connect(host, port))
            .await
            .map_err(|_| "connect_timeout")?
    }
    async fn connect(&self, host: &str, port: u16) -> Result<BoxStream> {
        let host = unbracket(host);
        let Some(proxy) = &self.url else {
            let stream = TcpStream::connect((host, port))
                .await
                .map_err(|_| "connect")?;
            stream.set_nodelay(true).map_err(|_| "socket")?;
            return Ok(Box::new(stream));
        };
        let proxy_address = (
            unbracket(proxy.host_str().unwrap()),
            proxy.port_or_known_default().unwrap_or(1080),
        );
        let user = decode(proxy.username())?;
        let password = decode(proxy.password().unwrap_or(""))?;
        if proxy.scheme().starts_with("socks4") {
            // tokio-socks 0.5.3 encodes SOCKS4a into a fixed 513-byte buffer.
            // Include the eight-byte header and both NUL terminators before calling it.
            if host.parse::<std::net::IpAddr>().is_err() && 10 + user.len() + host.len() > 513 {
                return Err("SOCKS4 用户名与目标域名组合过长（proxy_request_too_long）".into());
            }
            let stream = if user.is_empty() {
                tokio_socks::tcp::Socks4Stream::connect(proxy_address, (host, port)).await
            } else {
                tokio_socks::tcp::Socks4Stream::connect_with_userid(
                    proxy_address,
                    (host, port),
                    &user,
                )
                .await
            }
            .map_err(|_| "proxy_connect")?;
            let stream = stream.into_inner();
            stream.set_nodelay(true).map_err(|_| "socket")?;
            return Ok(Box::new(stream));
        }
        if proxy.scheme().starts_with("socks5") {
            let stream = if user.is_empty() {
                tokio_socks::tcp::Socks5Stream::connect(proxy_address, (host, port)).await
            } else {
                tokio_socks::tcp::Socks5Stream::connect_with_password(
                    proxy_address,
                    (host, port),
                    &user,
                    &password,
                )
                .await
            }
            .map_err(|_| "proxy_connect")?;
            let stream = stream.into_inner();
            stream.set_nodelay(true).map_err(|_| "socket")?;
            return Ok(Box::new(stream));
        }
        let tcp = TcpStream::connect(proxy_address)
            .await
            .map_err(|_| "proxy_connect")?;
        tcp.set_nodelay(true).map_err(|_| "socket")?;
        let stream: BoxStream = if proxy.scheme() == "https" {
            Box::new(self.secure(tcp, proxy.host_str().unwrap()).await?)
        } else {
            Box::new(tcp)
        };
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|_| "proxy_handshake")?;
        // The connection driver ends when CONNECT upgrades or when the sender/request is dropped.
        tokio::spawn(async move {
            let _ = connection.with_upgrades().await;
        });
        let authority = authority(host, port);
        let mut request = http::Request::builder()
            .method("CONNECT")
            .uri(&authority)
            .header("host", &authority);
        if !user.is_empty() {
            request = request.header(
                "proxy-authorization",
                format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
                ),
            );
        }
        let mut response = sender
            .send_request(
                request
                    .body(Empty::<Bytes>::new())
                    .map_err(|_| "proxy_request")?,
            )
            .await
            .map_err(|_| "proxy_connect")?;
        if !response.status().is_success() {
            return Err("proxy_rejected".into());
        }
        Ok(Box::new(TokioIo::new(
            hyper::upgrade::on(&mut response)
                .await
                .map_err(|_| "proxy_upgrade")?,
        )))
    }
    pub async fn secure<S: Stream>(
        &self,
        stream: S,
        host: &str,
    ) -> Result<tokio_rustls::client::TlsStream<S>> {
        self.secure_protocol(stream, host, false).await
    }
    pub async fn secure_protocol<S: Stream>(
        &self,
        stream: S,
        host: &str,
        h2: bool,
    ) -> Result<tokio_rustls::client::TlsStream<S>> {
        let name = rustls::pki_types::ServerName::try_from(unbracket(host).to_owned())
            .map_err(|_| "invalid TLS name")?;
        let mut config = (*self.tls).clone();
        config.alpn_protocols = if h2 {
            vec![b"h2".to_vec(), b"http/1.1".to_vec()]
        } else {
            vec![b"http/1.1".to_vec()]
        };
        tokio::time::timeout(
            Duration::from_secs(10),
            TlsConnector::from(Arc::new(config)).connect(name, stream),
        )
        .await
        .map_err(|_| "tls_timeout")?
        .map_err(|error| {
            if let Some(error) = error
                .get_ref()
                .and_then(|e| e.downcast_ref::<rustls::Error>())
            {
                match error {
                    rustls::Error::InvalidCertificate(_) => "tls_certificate",
                    _ => "tls_protocol",
                }
            } else if error.kind() == std::io::ErrorKind::UnexpectedEof {
                "tls_eof"
            } else {
                "tls"
            }
            .to_owned()
        })
    }
}
pub fn authority(host: &str, port: u16) -> String {
    let host = unbracket(host);
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}
fn decode(value: &str) -> Result<String> {
    let value = percent_encoding::percent_decode_str(value)
        .decode_utf8()
        .map_err(|_| "invalid proxy credentials")?
        .into_owned();
    Ok(value)
}

pub(crate) fn unbracket(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
}

/// Protocol rules shared by configuration validation and actual route construction.
pub fn validate_proxy_credentials(protocol: &str, user: &str, password: &str) -> Result<()> {
    let valid = match protocol {
        "http" | "https" => {
            !user.contains(':')
                && (password.is_empty() || !user.is_empty())
                && user
                    .bytes()
                    .chain(password.bytes())
                    .all(|b| (32..127).contains(&b))
        }
        "socks4" | "socks4a" => password.is_empty() && user.len() <= 255 && !user.contains('\0'),
        "socks5" | "socks5h" => {
            (user.is_empty() && password.is_empty())
                || ((1..=255).contains(&user.len()) && (1..=255).contains(&password.len()))
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(match protocol {
            "socks5" | "socks5h" => {
                "SOCKS5 用户名和密码须同时填写，长度各为 1–255 字节；或同时留空"
            }
            "socks4" | "socks4a" => "SOCKS4 用户名最多 255 字节，不能包含空字符，且不支持密码",
            _ => "HTTP 代理用户名不能包含冒号，凭据须为可打印 ASCII，填写密码时需填写用户名",
        }
        .into())
    }
}
