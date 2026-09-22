use gbf_flash_cache_core::{
    cache::Cache, certificates::Authority, engine::Engine, gateway::Gateway, network::Network,
    tunnel::Route,
};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
};
use url::Url;

async fn gateway(
    home: &std::path::Path,
    upstream: Option<&str>,
    roots: &[Vec<u8>],
) -> Arc<Gateway> {
    let ca = Authority::open(&home.join("ca")).unwrap();
    let cache = Cache::new(
        home.join("cache"),
        0,
        3600,
        Network::new(upstream, roots).unwrap(),
    )
    .unwrap();
    let engine = Engine::new(cache, Url::parse("https://example.org").unwrap()).unwrap();
    Gateway::start(0, engine, &ca, &[], Route::new(upstream, roots).unwrap())
        .await
        .unwrap()
}
async fn association(address: SocketAddr) -> (TcpStream, UdpSocket) {
    let mut control = TcpStream::connect(address).await.unwrap();
    control.write_all(&[5, 1, 0]).await.unwrap();
    let mut reply = [0; 2];
    control.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply, [5, 0]);
    control
        .write_all(&[5, 3, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .unwrap();
    let mut reply = [0; 10];
    control.read_exact(&mut reply).await.unwrap();
    assert_eq!(&reply[..4], &[5, 0, 0, 1]);
    let relay = SocketAddr::from((
        [reply[4], reply[5], reply[6], reply[7]],
        u16::from_be_bytes([reply[8], reply[9]]),
    ));
    let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    udp.connect(relay).await.unwrap();
    (control, udp)
}
fn packet(target: SocketAddr, data: &[u8]) -> Vec<u8> {
    let mut packet = vec![0, 0, 0];
    match target.ip() {
        std::net::IpAddr::V4(ip) => {
            packet.push(1);
            packet.extend(ip.octets());
        }
        std::net::IpAddr::V6(ip) => {
            packet.push(4);
            packet.extend(ip.octets());
        }
    }
    packet.extend(target.port().to_be_bytes());
    packet.extend(data);
    packet
}
async fn exchange(client: &UdpSocket, echo: &UdpSocket, data: &[u8]) {
    let sent = packet(echo.local_addr().unwrap(), data);
    client.send(&sent).await.unwrap();
    let mut buf = vec![0; 65535];
    let (n, source) = tokio::time::timeout(Duration::from_secs(2), echo.recv_from(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf[..n], data);
    echo.send_to(&buf[..n], source).await.unwrap();
    let n = tokio::time::timeout(Duration::from_secs(2), client.recv(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf[..n], sent);
}

#[tokio::test]
async fn mixed_gateway_tcp_udp_matrix_and_control_lifetime() {
    let parent_home = tempfile::tempdir().unwrap();
    let parent = gateway(parent_home.path(), None, &[]).await;
    let root = Authority::open(&parent_home.path().join("ca"))
        .unwrap()
        .certificate;
    for protocol in ["none", "http", "https", "socks4", "socks5"] {
        let home = tempfile::tempdir().unwrap();
        let url = format!("{protocol}://{}", parent.address);
        let g = gateway(
            home.path(),
            (protocol != "none").then_some(url.as_str()),
            &[root.clone()],
        )
        .await;
        // TCP works through each configured upstream protocol.
        let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target = tcp.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = tcp.accept().await.unwrap();
            let mut b = [0; 4];
            stream.read_exact(&mut b).await.unwrap();
            assert_eq!(&b, b"ping");
            stream.write_all(b"pong").await.unwrap();
        });
        let mut stream = tokio_socks::tcp::Socks5Stream::connect(g.address, target)
            .await
            .unwrap();
        stream.write_all(b"ping").await.unwrap();
        let mut b = [0; 4];
        stream.read_exact(&mut b).await.unwrap();
        assert_eq!(&b, b"pong");
        server.await.unwrap();
        let (control, client) = association(g.address).await;
        // A TCP-only upstream cannot affect the UDP direct route.
        let echo = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        for size in [0, 1, 16000] {
            exchange(&client, &echo, &vec![7; size]).await;
        }
        let other = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        exchange(&client, &other, b"second target").await;
        let ipv6 = UdpSocket::bind("[::1]:0").await.unwrap();
        exchange(&client, &ipv6, b"IPv6 target").await;
        assert_eq!(
            matches!(
                Route::new((protocol != "none").then_some(url.as_str()), &[])
                    .unwrap()
                    .udp_route()
                    .unwrap(),
                gbf_flash_cache_core::udp::Route::Socks { .. }
            ),
            protocol == "socks5"
        );
        let spoof = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        spoof
            .send_to(
                &packet(echo.local_addr().unwrap(), b"spoof"),
                client.peer_addr().unwrap(),
            )
            .await
            .unwrap();
        let mut buf = [0; 128];
        assert!(
            tokio::time::timeout(Duration::from_millis(100), echo.recv(&mut buf))
                .await
                .is_err()
        );
        let mut invalid = packet(echo.local_addr().unwrap(), b"fragment");
        invalid[2] = 1;
        client.send(&invalid).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), echo.recv(&mut buf))
                .await
                .is_err()
        );
        drop(control);
        tokio::time::sleep(Duration::from_millis(30)).await;
        let _ = client
            .send(&packet(echo.local_addr().unwrap(), b"closed"))
            .await;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), echo.recv(&mut buf))
                .await
                .is_err()
        );
        g.close().await;
    }
    parent.close().await;
}

#[tokio::test]
async fn socks5_rejection_never_falls_back_to_direct() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("socks5://{}", listener.local_addr().unwrap());
    let rejected = tokio::spawn(async move {
        let (mut control, _) = listener.accept().await.unwrap();
        let mut hello = [0; 3];
        control.read_exact(&mut hello).await.unwrap();
        control.write_all(&[5, 0]).await.unwrap();
        let mut command = [0; 10];
        control.read_exact(&mut command).await.unwrap();
        assert_eq!(&command[..3], &[5, 3, 0]);
        control.write_all(&[5, 7, 0]).await.unwrap();
    });
    let home = tempfile::tempdir().unwrap();
    let g = gateway(home.path(), Some(&url), &[]).await;
    let (_control, client) = association(g.address).await;
    let echo = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    client
        .send(&packet(echo.local_addr().unwrap(), b"must not bypass"))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), rejected)
        .await
        .unwrap()
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(150), echo.recv(&mut [0; 128]))
            .await
            .is_err()
    );
    g.close().await;
}
