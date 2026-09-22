//! Android TUN ingress: TCP uses the local gateway; UDP follows the configured egress.
#[path = "udp.rs"]
pub mod udp;
use gbf_flash_cache_core::trace::Trace;
use hickory_proto::{
    op::{Message, MessageType, ResponseCode},
    rr::{rdata::A, DNSClass, RData, Record, RecordType},
};
use ipstack::{IpStack, IpStackConfig, IpStackStream};
use std::{
    collections::HashMap,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Dns {
    names: Vec<String>,
    addresses: HashMap<String, Ipv4Addr>,
    trace: Option<Arc<Trace>>,
}
impl Dns {
    fn response(&mut self, packet: &[u8]) -> io::Result<Vec<u8>> {
        let query = Message::from_vec(packet).map_err(io::Error::other)?;
        let mut response = Message::new();
        response
            .set_id(query.id())
            .set_message_type(MessageType::Response)
            .set_recursion_desired(query.recursion_desired())
            .set_recursion_available(true);
        if query.message_type() != MessageType::Query || query.queries().len() != 1 {
            response.set_response_code(ResponseCode::FormErr);
        } else {
            let question = &query.queries()[0];
            response.add_query(question.clone());
            if let Some(log) = &self.trace { log.event("TUN_DNS", None, serde_json::json!({"host":question.name().to_ascii(),"record_type":question.query_type().to_string()})); }

            if question.query_class() != DNSClass::IN {
                response.set_response_code(ResponseCode::Refused);
            } else if question.query_type() == RecordType::A {
                let name = question
                    .name()
                    .to_ascii()
                    .trim_end_matches('.')
                    .to_ascii_lowercase();
                if !self.addresses.contains_key(&name) && self.names.len() >= 131_070 {
                    response.set_response_code(ResponseCode::ServFail);
                } else {
                    let address = *self.addresses.entry(name.clone()).or_insert_with(|| {
                        // Keep mappings stable for this capture lifetime; never recycle an in-use fake IP.
                        self.names.push(name);
                        Ipv4Addr::from(
                            u32::from(Ipv4Addr::new(198, 18, 0, 0)) + self.names.len() as u32,
                        )
                    });
                    response.add_answer(Record::from_rdata(
                        question.name().clone(),
                        5,
                        RData::A(A(address)),
                    ));
                }
            }
            // AAAA/HTTPS and other record types return NODATA; clients use the A mapping.
        }
        response.to_vec().map_err(io::Error::other)
    }
    fn authority(&self, peer: SocketAddr) -> io::Result<String> {
        if let IpAddr::V4(ip) = peer.ip() {
            let offset = u32::from(ip).wrapping_sub(u32::from(Ipv4Addr::new(198, 18, 0, 0)));
            if offset < 131_072 {
                let name = offset
                    .checked_sub(1)
                    .and_then(|index| self.names.get(index as usize))
                    .ok_or_else(|| io::Error::other("unknown virtual DNS address"))?;
                return Ok(format!("{name}:{}", peer.port()));
            }
        }
        Ok(peer.to_string())
    }
}
async fn connect(port: u16, authority: &str) -> io::Result<TcpStream> {
    let mut proxy = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await?;
    proxy.set_nodelay(true)?;
    proxy
        .write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes())
        .await?;
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= 8192 {
            return Err(io::Error::other("oversized local gateway response"));
        }
        header.push(proxy.read_u8().await?);
    }
    let status = header.split(|b| *b == b' ').nth(1);
    if status != Some(b"200".as_slice()) {
        return Err(io::Error::other("local gateway rejected connection"));
    }
    Ok(proxy)
}

pub async fn run<D>(
    device: D,
    port: u16,
    route: udp::Route,
    cancel: CancellationToken,
    trace: Option<Arc<Trace>>,
) -> io::Result<()>
where
    D: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut config = IpStackConfig::default();
    config
        .mtu(65535)
        .map_err(io::Error::other)?
        .udp_timeout(Duration::from_secs(60));
    let mut stack = IpStack::new(config, device);
    let dns = Arc::new(Mutex::new(Dns { trace: trace.clone(), ..Dns::default() }));
    let mut tasks = JoinSet::new();
    loop {
        let stream = tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tasks.join_next(), if !tasks.is_empty() => continue,
            stream = stack.accept() => stream.map_err(io::Error::other)?,
        };
        if tasks.len() >= 4096 {
            drop(stream);
            continue;
        }
        if let Some(log) = &trace {
            let (protocol, peer) = match &stream {
                IpStackStream::Tcp(s) => ("TCP", s.peer_addr()),
                IpStackStream::Udp(s) => ("UDP", s.peer_addr()),
                _ => continue,
            };
            let target = if peer.port() == 53 { Some(peer.to_string()) } else { dns.lock().ok().and_then(|dns| dns.authority(peer).ok()) };
            log.event("TUN_FLOW", None, serde_json::json!({"protocol":protocol,"target":target,"port":peer.port(),"action":if peer.port()==53 {"dns"} else if protocol=="UDP" {"relay_uncached"} else {"gateway"}}));
        }
        match stream {
            IpStackStream::Tcp(mut stream) => {
                let dns = dns.clone();
                let peer = stream.peer_addr();
                tasks.spawn(async move {
                    if peer.port() == 53 {
                        loop {
                            let length = stream.read_u16().await? as usize;
                            let mut packet = vec![0; length];
                            stream.read_exact(&mut packet).await?;
                            let answer = dns
                                .lock()
                                .map_err(|_| io::Error::other("DNS unavailable"))?
                                .response(&packet)?;
                            stream.write_u16(answer.len() as u16).await?;
                            stream.write_all(&answer).await?;
                        }
                    }
                    let authority = dns
                        .lock()
                        .map_err(|_| io::Error::other("DNS unavailable"))?
                        .authority(peer)?;
                    let mut proxy =
                        tokio::time::timeout(Duration::from_secs(30), connect(port, &authority))
                            .await??;
                    tokio::io::copy_bidirectional(&mut stream, &mut proxy).await?;
                    Ok::<_, io::Error>(())
                });
            }
            IpStackStream::Udp(mut stream) if stream.peer_addr().port() == 53 => {
                let dns = dns.clone();
                tasks.spawn(async move {
                    let mut packet = vec![0; 65535];
                    loop {
                        let size = stream.read(&mut packet).await?;
                        if size == 0 {
                            return Ok(());
                        }
                        let answer = dns
                            .lock()
                            .map_err(|_| io::Error::other("DNS unavailable"))?
                            .response(&packet[..size])?;
                        stream.write_all(&answer).await?;
                    }
                });
            }
            IpStackStream::Udp(mut stream) => {
                let dns = dns.clone();
                let route = route.clone();
                tasks.spawn(async move {
                    let peer = stream.peer_addr();
                    let authority = dns.lock().map_err(|_| io::Error::other("DNS unavailable"))?.authority(peer)?;
                    let mut remote = route.open(Some(&authority)).await?;
                    let mut outbound = vec![0;65535];
                    let mut inbound = vec![0;65535];
                    loop {
                        tokio::select! {
                            size=stream.read(&mut outbound)=>remote.send(&outbound[..size?]).await?,
                            size=remote.recv(&mut inbound)=>{
                                let size=size?;
                                // ipstack truncates replies above its MTU. Never deliver partial datagrams.
                                let maximum=if peer.is_ipv4(){65507}else{65487};
                                if size>maximum{return Err(io::Error::other("UDP datagram exceeds tunnel MTU"));}
                                if stream.write(&inbound[..size]).await? != size {return Err(io::Error::other("short UDP write"));}
                            }
                        }
                    }
                    #[allow(unreachable_code)]
                    Ok::<_,io::Error>(())
                });
            }
            _ => {} // Non-TCP/UDP transports are not proxy protocols.
        }
    }
    drop(stack);
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::{op::Query, rr::Name};
    fn checksum(bytes: &[u8]) -> u16 {
        let mut sum: u32 = bytes
            .chunks(2)
            .map(|b| u16::from_be_bytes([b[0], *b.get(1).unwrap_or(&0)]) as u32)
            .sum();
        while sum >> 16 != 0 {
            sum = (sum & 65535) + (sum >> 16);
        }
        !(sum as u16)
    }
    fn packet(seq: u32, ack: u32, flags: u8, data: &[u8]) -> Vec<u8> {
        let mut p = vec![0u8; 40];
        p[0] = 0x45;
        p[8] = 64;
        p[9] = 6;
        p[2..4].copy_from_slice(&((40 + data.len()) as u16).to_be_bytes());
        p[12..16].copy_from_slice(&[10, 254, 254, 1]);
        p[16..20].copy_from_slice(&[192, 0, 2, 1]);
        p[20..22].copy_from_slice(&53001u16.to_be_bytes());
        p[22..24].copy_from_slice(&443u16.to_be_bytes());
        p[24..28].copy_from_slice(&seq.to_be_bytes());
        p[28..32].copy_from_slice(&ack.to_be_bytes());
        p[32] = 0x50;
        p[33] = flags;
        p[34..36].copy_from_slice(&65535u16.to_be_bytes());
        let sum = checksum(&p[..20]);
        p[10..12].copy_from_slice(&sum.to_be_bytes());
        p.extend(data);
        let mut pseudo = p[12..20].to_vec();
        pseudo.extend([0, 6]);
        pseudo.extend(((20 + data.len()) as u16).to_be_bytes());
        pseudo.extend(&p[20..]);
        let sum = checksum(&pseudo);
        p[36..38].copy_from_slice(&sum.to_be_bytes());
        p
    }
    async fn receive(client: &mut tokio::io::DuplexStream) -> Vec<u8> {
        let mut p = vec![0u8; 20];
        client.read_exact(&mut p).await.unwrap();
        let length = u16::from_be_bytes([p[2], p[3]]) as usize;
        p.resize(length, 0);
        client.read_exact(&mut p[20..]).await.unwrap();
        p
    }
    #[test]
    fn dns_preserves_names_and_never_reuses_unknown_addresses() {
        let mut dns = Dns::default();
        let mut query = Message::new();
        query.set_id(42).add_query(Query::query(
            Name::from_ascii("GAME.granbluefantasy.jp.").unwrap(),
            RecordType::A,
        ));
        let packet = query.to_vec().unwrap();
        let answer = Message::from_vec(&dns.response(&packet).unwrap()).unwrap();
        assert_eq!(answer.id(), 42);
        assert_eq!(answer.answers().len(), 1);
        assert_eq!(
            dns.authority("198.18.0.1:443".parse().unwrap()).unwrap(),
            "game.granbluefantasy.jp:443"
        );
        assert!(dns.authority("198.18.0.2:443".parse().unwrap()).is_err());
        dns.response(&packet).unwrap();
        assert_eq!(dns.names.len(), 1);
        assert_eq!(
            dns.authority("[::1]:443".parse().unwrap()).unwrap(),
            "[::1]:443"
        );
        assert!(dns.response(&[0, 1, 2]).is_err());
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_with_pending_tcp_connections() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (device, mut client) = tokio::io::duplex(65536);
        let cancel = CancellationToken::new();
        let task = tokio::spawn(run(device, port, udp::Route::Direct, cancel.clone(), None));
        for i in 0..256u16 {
            let mut syn = packet(1000, 0, 2, &[]);
            syn[20..22].copy_from_slice(&(40000+i).to_be_bytes());
            syn[36..38].fill(0);
            let mut pseudo = syn[12..20].to_vec();
            pseudo.extend([0, 6, 0, 20]);
            pseudo.extend(&syn[20..]);
            syn[36..38].copy_from_slice(&checksum(&pseudo).to_be_bytes());
            client.write_all(&syn).await.unwrap();
            let response = receive(&mut client).await;
            assert_eq!(response[33] & 0x12, 0x12);
        }
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(3), task).await.expect("stop stuck").unwrap().unwrap();

    }
    #[tokio::test]
    async fn tun_dns_packet_and_shutdown_without_a_device() {
        let home = tempfile::tempdir().unwrap();
        let trace = Trace::open(&home.path().join("tun.jsonl")).unwrap();
        let (device, mut client) = tokio::io::duplex(65536);
        let cancel = CancellationToken::new();
        let task = tokio::spawn(run(device, 8765, udp::Route::Direct, cancel.clone(), Some(trace.clone())));
        let mut query = Message::new();
        query.set_id(123).add_query(Query::query(
            Name::from_ascii("game.granbluefantasy.jp").unwrap(),
            RecordType::A,
        ));
        let payload = query.to_vec().unwrap();
        let mut packet = vec![0u8; 28];
        packet[0] = 0x45;
        let length = (28 + payload.len()) as u16;
        packet[2..4].copy_from_slice(&length.to_be_bytes());
        packet[8] = 64;
        packet[9] = 17;
        packet[12..16].copy_from_slice(&[10, 254, 254, 1]);
        packet[16..20].copy_from_slice(&[198, 18, 0, 1]);
        packet[20..22].copy_from_slice(&53000u16.to_be_bytes());
        packet[22..24].copy_from_slice(&53u16.to_be_bytes());
        packet[24..26].copy_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
        let mut checksum: u32 = packet[..20]
            .chunks_exact(2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]) as u32)
            .sum();
        while checksum >> 16 != 0 {
            checksum = (checksum & 65535) + (checksum >> 16);
        }
        packet[10..12].copy_from_slice(&(!(checksum as u16)).to_be_bytes());
        packet.extend(payload);
        client.write_all(&packet).await.unwrap();
        let mut response = vec![0; 65536];
        let size = tokio::time::timeout(Duration::from_secs(2), client.read(&mut response))
            .await
            .unwrap()
            .unwrap();
        let answer = Message::from_vec(&response[28..size]).unwrap();
        assert_eq!(answer.id(), 123);
        assert_eq!(answer.answers().len(), 1);
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        trace.close().await;
        let text = std::fs::read_to_string(home.path().join("tun.jsonl")).unwrap();
        let rows: Vec<serde_json::Value> = text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert!(rows.iter().any(|r| r["event"] == "TUN_DNS" && r["host"] == "game.granbluefantasy.jp."));
        assert!(rows.iter().any(|r| r["event"] == "TUN_FLOW" && r["protocol"] == "UDP" && r["action"] == "dns"));
    }
    #[tokio::test]
    async fn tun_udp_ipv4_ipv6_large_datagram_and_shutdown() {
        for (address, protocol) in ["127.0.0.1:0", "[::1]:0"].into_iter()
            .flat_map(|address| ["none", "HTTP", "HTTPS", "SOCKS4"].into_iter().map(move |protocol| (address, protocol))) {
            let echo = tokio::net::UdpSocket::bind(address).await.unwrap();
            let peer = echo.local_addr().unwrap();
            let echoed = tokio::spawn(async move {
                let mut bytes = vec![0; 65535];
                let (n, source) = echo.recv_from(&mut bytes).await.unwrap();
                echo.send_to(&bytes[..n], source).await.unwrap();
            });
            let (device, mut client) = tokio::io::duplex(131072);
            let cancel = CancellationToken::new();
            let fields = std::collections::BTreeMap::from([
                ("proxy".into(), (protocol != "none").to_string()),
                ("protocol".into(), protocol.into()),
            ]);
            let route = udp::from_fields(&fields).unwrap();
            let task = tokio::spawn(run(device, 8765, route, cancel.clone(), None));
            let data = vec![37; 16000];
            let h = if peer.is_ipv4() { 20 } else { 40 };
            let mut packet = vec![0u8; h + 8];
            if let IpAddr::V4(ip) = peer.ip() {
                packet[0] = 0x45;
                packet[2..4].copy_from_slice(&((h + 8 + data.len()) as u16).to_be_bytes());
                packet[8] = 64;
                packet[9] = 17;
                packet[12..16].copy_from_slice(&[10, 254, 254, 1]);
                packet[16..20].copy_from_slice(&ip.octets());
                let mut sum: u32 = packet[..20]
                    .chunks_exact(2)
                    .map(|b| u16::from_be_bytes([b[0], b[1]]) as u32)
                    .sum();
                while sum >> 16 != 0 {
                    sum = (sum & 65535) + (sum >> 16);
                }
                packet[10..12].copy_from_slice(&(!(sum as u16)).to_be_bytes());
            } else if let IpAddr::V6(ip) = peer.ip() {
                packet[0] = 0x60;
                packet[4..6].copy_from_slice(&((8 + data.len()) as u16).to_be_bytes());
                packet[6] = 17;
                packet[7] = 64;
                packet[8..24].copy_from_slice(
                    &"fd7a:6766:6300::1"
                        .parse::<std::net::Ipv6Addr>()
                        .unwrap()
                        .octets(),
                );
                packet[24..40].copy_from_slice(&ip.octets());
            }
            packet[h..h + 2].copy_from_slice(&53100u16.to_be_bytes());
            packet[h + 2..h + 4].copy_from_slice(&peer.port().to_be_bytes());
            packet[h + 4..h + 6].copy_from_slice(&((8 + data.len()) as u16).to_be_bytes());
            packet.extend(&data);
            if peer.is_ipv6() {
                let mut pseudo = packet[8..40].to_vec();
                pseudo.extend(((8 + data.len()) as u32).to_be_bytes());
                pseudo.extend([0, 0, 0, 17]);
                pseudo.extend(&packet[40..]);
                let mut sum: u32 = pseudo
                    .chunks(2)
                    .map(|b| u16::from_be_bytes([b[0], *b.get(1).unwrap_or(&0)]) as u32)
                    .sum();
                while sum >> 16 != 0 {
                    sum = (sum & 65535) + (sum >> 16);
                }
                packet[h + 6..h + 8].copy_from_slice(&(!(sum as u16)).to_be_bytes());
            }
            client.write_all(&packet).await.unwrap();
            let mut response = vec![0; 65535];
            let n = tokio::time::timeout(Duration::from_secs(2), client.read(&mut response))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(&response[h + 8..n], data);
            echoed.await.unwrap();
            cancel.cancel();
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }
    }
    #[tokio::test]
    async fn tun_tcp_reaches_gateway_and_returns_payload() {
        use tokio::net::TcpListener;
        tokio::time::timeout(Duration::from_secs(3), async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                let (mut connection, _) = listener.accept().await.unwrap();
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    header.push(connection.read_u8().await.unwrap());
                }
                assert!(String::from_utf8(header)
                    .unwrap()
                    .starts_with("CONNECT 192.0.2.1:443 HTTP/1.1"));
                connection
                    .write_all(b"HTTP/1.1 200 OK\r\n\r\n")
                    .await
                    .unwrap();
                let mut request = [0; 4];
                connection.read_exact(&mut request).await.unwrap();
                assert_eq!(&request, b"ping");
                connection.write_all(b"pong").await.unwrap();
            });
            let (device, mut client) = tokio::io::duplex(65536);
            let cancel = CancellationToken::new();
            let task = tokio::spawn(run(device, port, udp::Route::Direct, cancel.clone(), None));
            client.write_all(&packet(1000, 0, 2, &[])).await.unwrap();
            let syn = receive(&mut client).await;
            assert_eq!(syn[33] & 0x12, 0x12);
            let ack = u32::from_be_bytes(syn[24..28].try_into().unwrap()).wrapping_add(1);
            client
                .write_all(&packet(1001, ack, 0x18, b"ping"))
                .await
                .unwrap();
            loop {
                let response = receive(&mut client).await;
                let offset = 20 + ((response[32] >> 4) as usize) * 4;
                if response.len() > offset {
                    assert_eq!(&response[offset..], b"pong");
                    break;
                }
            }
            server.await.unwrap();
            cancel.cancel();
            task.await.unwrap().unwrap();
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn local_connect_keeps_first_payload_and_rejects_failed_gateway() {
        use tokio::net::TcpListener;
        for status in [200, 502] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    header.push(stream.read_u8().await.unwrap());
                }
                assert_eq!(String::from_utf8(header).unwrap(),"CONNECT game.granbluefantasy.jp:443 HTTP/1.1\r\nHost: game.granbluefantasy.jp:443\r\n\r\n");
                stream
                    .write_all(format!("HTTP/1.1 {status} Result\r\n\r\nhello").as_bytes())
                    .await
                    .unwrap();
            });
            let connection = connect(port, "game.granbluefantasy.jp:443").await;
            if status == 200 {
                let mut bytes = [0; 5];
                connection.unwrap().read_exact(&mut bytes).await.unwrap();
                assert_eq!(&bytes, b"hello");
            } else {
                assert!(connection.is_err());
            }
            server.await.unwrap();
        }
    }
}
