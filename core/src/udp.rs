//! Shared UDP transport. A SOCKS5 failure never changes the selected route.
use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
};

#[derive(Clone, Default)]
pub enum Route {
    #[default]
    Direct,
    Socks {
        host: String,
        port: u16,
        user: String,
        password: String,
    },
}
impl Route {
    pub async fn open(&self, destination: Option<&str>) -> io::Result<Transport> {
        tokio::time::timeout(Duration::from_secs(10), self.connect(destination))
            .await
            .map_err(|_| io::Error::other("UDP 上游连接超时"))?
    }
    async fn connect(&self, destination: Option<&str>) -> io::Result<Transport> {
        match self {
            Self::Direct => {
                let address = tokio::net::lookup_host(
                    destination.ok_or_else(|| io::Error::other("missing UDP destination"))?,
                )
                .await?
                .next()
                .ok_or_else(|| io::Error::other("UDP destination unresolved"))?;
                let socket = bind(address).await?;
                socket.connect(address).await?;
                Ok(Transport {
                    destination: destination.map(str::to_owned),
                    socket,
                    control: None,
                    header: Vec::new(),
                })
            }
            Self::Socks {
                host,
                port,
                user,
                password,
            } => {
                let mut control = TcpStream::connect((host.as_str(), *port)).await?;
                let method = if user.is_empty() { 0 } else { 2 };
                control.write_all(&[5, 1, method]).await?;
                let mut reply = [0; 2];
                control.read_exact(&mut reply).await?;
                if reply != [5, method] {
                    return Err(io::Error::other("SOCKS5 authentication method rejected"));
                }
                if method == 2 {
                    let mut auth = vec![1, user.len() as u8];
                    auth.extend(user.as_bytes());
                    auth.push(password.len() as u8);
                    auth.extend(password.as_bytes());
                    control.write_all(&auth).await?;
                    control.read_exact(&mut reply).await?;
                    if reply != [1, 0] {
                        return Err(io::Error::other("SOCKS5 authentication failed"));
                    }
                }
                let peer = control.peer_addr()?;
                let socket = bind(peer).await?;
                // RFC1928: unspecified source address, actual local UDP port.
                let local = socket.local_addr()?;
                let mut request = vec![5, 3, 0];
                request.extend(encode(&local.to_string())?);
                control.write_all(&request).await?;
                let mut prefix = [0; 3];
                control.read_exact(&mut prefix).await?;
                if prefix != [5, 0, 0] {
                    return Err(io::Error::other("上游拒绝 SOCKS5 UDP ASSOCIATE"));
                }
                let relay = read_address(&mut control).await?;
                let mut address = tokio::net::lookup_host(relay.as_str())
                    .await?
                    .next()
                    .ok_or_else(|| io::Error::other("UDP relay unresolved"))?;
                if address.ip().is_unspecified() {
                    address.set_ip(peer.ip());
                }
                if address.port() == 0 {
                    return Err(io::Error::other("invalid UDP relay port"));
                }
                socket.connect(address).await?;
                let mut header = vec![0, 0, 0];
                if let Some(destination) = destination {
                    header.extend(encode(destination)?);
                }
                Ok(Transport {
                    destination: destination.map(str::to_owned),
                    socket,
                    control: Some(control),
                    header,
                })
            }
        }
    }
}
async fn bind(peer: SocketAddr) -> io::Result<UdpSocket> {
    UdpSocket::bind(if peer.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    })
    .await
}
pub(crate) fn encode(destination: &str) -> io::Result<Vec<u8>> {
    if let Ok(address) = destination.parse::<SocketAddr>() {
        let mut bytes = match address.ip() {
            IpAddr::V4(ip) => {
                let mut b = vec![1];
                b.extend(ip.octets());
                b
            }
            IpAddr::V6(ip) => {
                let mut b = vec![4];
                b.extend(ip.octets());
                b
            }
        };
        bytes.extend(address.port().to_be_bytes());
        return Ok(bytes);
    }
    let (host, port) = destination
        .rsplit_once(':')
        .ok_or_else(|| io::Error::other("invalid UDP target"))?;
    let port = port.parse::<u16>().map_err(io::Error::other)?;
    if host.is_empty() || host.len() > 255 {
        return Err(io::Error::other("invalid UDP hostname"));
    }
    let mut bytes = vec![3, host.len() as u8];
    bytes.extend(host.as_bytes());
    bytes.extend(port.to_be_bytes());
    Ok(bytes)
}
async fn read_address(stream: &mut (impl tokio::io::AsyncRead + Unpin)) -> io::Result<String> {
    let kind = stream.read_u8().await?;
    let host = match kind {
        1 => {
            let mut b = [0; 4];
            stream.read_exact(&mut b).await?;
            Ipv4Addr::from(b).to_string()
        }
        4 => {
            let mut b = [0; 16];
            stream.read_exact(&mut b).await?;
            format!("[{}]", Ipv6Addr::from(b))
        }
        3 => {
            let n = stream.read_u8().await? as usize;
            if n == 0 {
                return Err(io::Error::other("empty relay host"));
            }
            let mut b = vec![0; n];
            stream.read_exact(&mut b).await?;
            String::from_utf8(b).map_err(io::Error::other)?
        }
        _ => return Err(io::Error::other("invalid SOCKS5 address")),
    };
    Ok(format!("{host}:{}", stream.read_u16().await?))
}
fn payload(packet: &[u8]) -> io::Result<&[u8]> {
    if packet.get(..3) != Some(&[0, 0, 0]) {
        return Err(io::Error::other("invalid or fragmented SOCKS5 UDP packet"));
    }
    let offset = match packet.get(3) {
        Some(1) => 10,
        Some(4) => 22,
        Some(3) => {
            7 + *packet
                .get(4)
                .ok_or_else(|| io::Error::other("short UDP header"))? as usize
        }
        _ => return Err(io::Error::other("invalid UDP address type")),
    };
    packet
        .get(offset..)
        .ok_or_else(|| io::Error::other("short UDP packet"))
}
pub struct Transport {
    socket: UdpSocket,
    control: Option<TcpStream>,
    header: Vec<u8>,
    destination: Option<String>,
}
impl Transport {
    pub async fn send(&self, data: &[u8]) -> io::Result<()> {
        if self.control.is_some() {
            let mut packet = self.header.clone();
            packet.extend(data);
            self.socket.send(&packet).await?;
        } else {
            self.socket.send(data).await?;
        }
        Ok(())
    }
    // Fixed-target consumers (Android TUN) cannot change the source of their stream.
    pub async fn recv(&mut self, data: &mut [u8]) -> io::Result<usize> {
        let (size, source) = self.recv_from(data).await?;
        if let Some(target) = &self.destination {
            if !matching_source(target, &source) {
                return Err(io::Error::other("SOCKS5 UDP response source mismatch"));
            }
        }
        Ok(size)
    }
    pub async fn recv_from(&mut self, data: &mut [u8]) -> io::Result<(usize, String)> {
        let size = if let Some(control) = &mut self.control {
            let mut closed = [0];
            tokio::select! {
                result=self.socket.recv(data)=>result?,
                _=control.read(&mut closed)=>return Err(io::Error::other("SOCKS5 UDP control connection closed")),
            }
        } else {
            self.socket.recv(data).await?
        };
        if self.control.is_none() {
            return Ok((size, self.socket.peer_addr()?.to_string()));
        }
        let (source, body) = datagram(&data[..size]).await?;
        let length = body.len();
        let offset = size - length;
        data.copy_within(offset..size, 0);
        Ok((length, source))
    }
}

fn matching_source(target: &str, source: &str) -> bool {
    match (target.parse::<SocketAddr>(), source.parse::<SocketAddr>()) {
        (Ok(target), Ok(source)) => target == source,
        (Ok(_), Err(_)) => false,
        (Err(_), _) => {
            let Some((host, port)) = target.rsplit_once(':') else { return false };
            let Some((source_host, source_port)) = source.rsplit_once(':') else { return false };
            // Remote DNS belongs to the SOCKS server. Do not resolve locally and
            // reject a valid remote answer merely because local DNS differs.
            port == source_port && (source.parse::<SocketAddr>().is_ok() || host.eq_ignore_ascii_case(source_host))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    #[test]
    fn fixed_target_source_matching() {
        assert!(matching_source("192.0.2.1:53", "192.0.2.1:53"));
        assert!(!matching_source("192.0.2.1:53", "203.0.113.99:53"));
        assert!(!matching_source("192.0.2.1:53", "192.0.2.1:54"));
        assert!(matching_source("[::1]:53", "[0:0:0:0:0:0:0:1]:53"));
        assert!(matching_source("EXAMPLE.org:443", "example.org:443"));
        assert!(matching_source("example.org:443", "203.0.113.1:443"));
        assert!(!matching_source("example.org:443", "other.org:443"));
        assert!(!matching_source("example.org:443", "203.0.113.1:54"));
    }
    #[test]
    fn settings_and_packet_bounds() {
        for address in ["127.0.0.1:443", "[::1]:443", "example.org:443"] {
            let mut packet = vec![0, 0, 0];
            packet.extend(encode(address).unwrap());
            packet.extend(b"body");
            assert_eq!(payload(&packet).unwrap(), b"body");
            for n in 0..packet.len() - 4 {
                assert!(payload(&packet[..n]).is_err());
            }
            packet[2] = 1;
            assert!(payload(&packet).is_err());
        }
    }
    #[tokio::test]
    async fn direct_ipv4_ipv6_preserves_datagrams() {
        for address in ["127.0.0.1:0", "[::1]:0"] {
            let echo = UdpSocket::bind(address).await.unwrap();
            let target = echo.local_addr().unwrap().to_string();
            let task = tokio::spawn(async move {
                let mut b = vec![0; 65535];
                for _ in 0..4 {
                    let (n, peer) = echo.recv_from(&mut b).await.unwrap();
                    echo.send_to(&b[..n], peer).await.unwrap();
                }
            });
            let mut client = Route::Direct.open(Some(&target)).await.unwrap();
            for n in [0, 1, 16000, 60000] {
                let data = vec![n as u8; n];
                client.send(&data).await.unwrap();
                let mut b = vec![0; 65535];
                let n = tokio::time::timeout(Duration::from_secs(2), client.recv(&mut b))
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(&b[..n], data);
            }
            task.await.unwrap();
        }
    }
    #[tokio::test]
    async fn socks_association_auth_rejection_and_control_lifetime() {
        for (authenticated, rejected) in [(false, false), (true, false), (false, true)] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let relay = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let relay_port = relay.local_addr().unwrap().port();
            let (close_tx, close_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut control, _) = listener.accept().await.unwrap();
                let mut hello = [0; 3];
                control.read_exact(&mut hello).await.unwrap();
                assert_eq!(hello, [5, 1, if authenticated { 2 } else { 0 }]);
                control.write_all(&[5, hello[2]]).await.unwrap();
                if authenticated {
                    assert_eq!(control.read_u8().await.unwrap(), 1);
                    let n = control.read_u8().await.unwrap() as usize;
                    let mut user = vec![0; n];
                    control.read_exact(&mut user).await.unwrap();
                    assert_eq!(user, b"u%41:");
                    let n = control.read_u8().await.unwrap() as usize;
                    let mut pass = vec![0; n];
                    control.read_exact(&mut pass).await.unwrap();
                    assert_eq!(pass, b"p%42");
                    control.write_all(&[1, 0]).await.unwrap();
                }
                let mut command = [0; 3];
                control.read_exact(&mut command).await.unwrap();
                assert_eq!(command, [5, 3, 0]);
                let client = read_address(&mut control).await.unwrap();
                assert_ne!(client, "0.0.0.0:0");
                if rejected {
                    control.write_all(&[5, 7, 0]).await.unwrap();
                    return;
                }
                let mut response = vec![5, 0, 0, 1, 0, 0, 0, 0];
                response.extend(relay_port.to_be_bytes());
                control.write_all(&response).await.unwrap();
                let mut b = vec![0; 65535];
                for _ in 0..3 {
                    let (n, peer) = relay.recv_from(&mut b).await.unwrap();
                    assert_eq!(
                        &b[3..3 + encode("example.org:443").unwrap().len()],
                        encode("example.org:443").unwrap()
                    );
                    relay.send_to(&b[..n], peer).await.unwrap();
                }
                let _ = close_rx.await;
            });
            let route = Route::Socks {
                host: "127.0.0.1".into(),
                port,
                user: if authenticated {
                    "u%41:".into()
                } else {
                    String::new()
                },
                password: if authenticated {
                    "p%42".into()
                } else {
                    String::new()
                },
            };
            let result = route.open(Some("example.org:443")).await;
            if rejected {
                assert!(result.is_err());
                server.await.unwrap();
                continue;
            }
            let mut client = result.unwrap();
            for n in [0, 5, 16000] {
                let data = vec![17; n];
                client.send(&data).await.unwrap();
                let mut b = vec![0; 65535];
                let n = tokio::time::timeout(Duration::from_secs(2), client.recv(&mut b))
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(&b[..n], data);
            }
            close_tx.send(()).unwrap();
            let mut b = [0; 8];
            assert!(
                tokio::time::timeout(Duration::from_secs(2), client.recv(&mut b))
                    .await
                    .unwrap()
                    .is_err()
            );
            server.await.unwrap();
        }
    }
}

// One association is tied to its TCP peer and ends when that control connection closes.
pub(crate) async fn associate(
    mut control: TcpStream,
    client_host: &str,
    client_port: u16,
    route: Route,
) -> io::Result<()> {
    use std::{collections::HashMap, sync::Arc};
    use tokio::{sync::mpsc, task::JoinSet};
    let peer = control.peer_addr()?;
    let requested = client_host
        .trim_matches(['[', ']'])
        .parse::<IpAddr>()
        .map_err(|_| io::Error::other("UDP association requires a client IP"))?;
    if !requested.is_unspecified() && requested != peer.ip() {
        control.write_all(&[5, 2, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        return Err(io::Error::other("UDP client does not match TCP peer"));
    }
    let relay = Arc::new(UdpSocket::bind((control.local_addr()?.ip(), 0)).await?);
    let local = relay.local_addr()?;
    let mut reply = vec![5, 0, 0];
    reply.extend(encode(&local.to_string())?);
    control.write_all(&reply).await?;
    let mut endpoint = (client_port != 0).then_some(SocketAddr::new(peer.ip(), client_port));
    let mut flows = HashMap::<String, mpsc::Sender<Vec<u8>>>::new();
    let mut tasks = JoinSet::new();
    let mut packet = vec![0; 65535];
    let mut closed = [0; 1];
    loop {
        tokio::select! {
            biased;
            _ = control.read(&mut closed) => return Ok(()),
            Some(completed) = tasks.join_next(), if !tasks.is_empty() => {
                if let Ok(target) = completed { flows.remove(&target); }
            }
            result = relay.recv_from(&mut packet) => {
                let (n, source) = result?;
                if source.ip() != peer.ip() || endpoint.is_some_and(|e| e != source) {
                    continue;
                }
                let Ok((target, data)) = datagram(&packet[..n]).await else { continue };
                endpoint = Some(source);
                if let Some(tx) = flows.get(&target) {
                    let _ = tx.try_send(data.to_vec());
                    continue;
                }
                // Bound per-client resources; idle flows expire, excess UDP is dropped.
                if flows.len() >= 64 { continue; }
                let (tx, mut rx) = mpsc::channel::<Vec<u8>>(16);
                let _ = tx.try_send(data.to_vec());
                flows.insert(target.clone(), tx);
                let route = route.clone();
                let relay = relay.clone();
                tasks.spawn(async move {
                    let _ = async {
                        let (host, port) = target.rsplit_once(':').ok_or_else(|| io::Error::other("invalid UDP target"))?;
                        crate::tunnel::check_loop(host, port.parse().map_err(io::Error::other)?, local)
                            .await.map_err(|e| io::Error::other(e.to_string()))?;
                        let mut remote = route.open(Some(&target)).await?;

                        let mut incoming = vec![0; 65535];
                        loop {
                            tokio::select! {
                                Some(data) = rx.recv() => remote.send(&data).await?,
                                n = remote.recv_from(&mut incoming) => {
                                    let (n, origin) = n?;
                                    let mut response = vec![0, 0, 0];
                                    response.extend(encode(&origin)?);
                                    response.extend_from_slice(&incoming[..n]);
                                    relay.send_to(&response, source).await?;
                                }
                                _ = tokio::time::sleep(Duration::from_secs(60)) => return Ok::<(), io::Error>(()),
                            }
                        }
                    }.await;
                    target
                });
            }
        }
    }
}

async fn datagram(packet: &[u8]) -> io::Result<(String, &[u8])> {
    payload(packet)?;
    let mut data = &packet[3..];
    let target = read_address(&mut data).await?;
    let (host, port) = target
        .rsplit_once(':')
        .ok_or_else(|| io::Error::other("invalid UDP target"))?;
    if port == "0"
        || host.is_empty()
        || host.len() > 255
        || !host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-_:[]".contains(&c))
    {
        return Err(io::Error::other("invalid UDP target"));
    }
    Ok((target, data))
}
