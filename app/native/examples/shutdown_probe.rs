use gbf_flash_cache_app::application::{Fields, Service};
use std::time::Instant;
#[tokio::main]
async fn main() {
    let home = tempfile::tempdir().unwrap();
    let mut service = Service::open(home.path().into()).unwrap();
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    service
        .command("start", Fields::from([("port".into(), port.to_string())]))
        .await
        .unwrap();
    for i in 0..3000 {
        std::fs::write(home.path().join(format!("logs/{i}.jsonl")), b"{}\n").unwrap();
    }
    let start = Instant::now();
    service.command("stop", Fields::new()).await.unwrap();
    println!("stop_ms={:.3}", start.elapsed().as_secs_f64() * 1000.);
    let start = Instant::now();
    service.close().await.unwrap();
    println!(
        "close_after_stop_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.
    );
    drop(service);
    let start = Instant::now();
    let _service = Service::open(home.path().into()).unwrap();
    println!("next_open_ms={:.3}", start.elapsed().as_secs_f64() * 1000.);
}
