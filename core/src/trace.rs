use crate::hash;
use serde_json::{json, Value};
use std::{
    io::{BufWriter, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
use tokio::sync::mpsc;

/// Event metadata only. No cookies, request bodies, response bodies or query values.
pub struct Trace {
    sender: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    zero: Instant,
    pub dropped: AtomicU64,
    pub failed: Arc<AtomicBool>,
}
impl Trace {
    pub fn open(path: &Path) -> std::io::Result<Arc<Self>> {
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?;
        let (sender, mut receiver) = mpsc::channel::<Vec<u8>>(8192);
        let failed = Arc::new(AtomicBool::new(false));
        let errors = failed.clone();
        let worker = std::thread::spawn(move || {
            let mut file = BufWriter::new(file);
            while let Some(mut bytes) = receiver.blocking_recv() {
                bytes.push(b'\n');
                if file.write_all(&bytes).is_err() {
                    errors.store(true, Ordering::Relaxed);
                }
            }
            if file.flush().is_err() {
                errors.store(true, Ordering::Relaxed);
            }
        });
        Ok(Arc::new(Self {
            sender: Mutex::new(Some(sender)),
            worker: Mutex::new(Some(worker)),
            zero: Instant::now(),
            dropped: AtomicU64::new(0),
            failed,
        }))
    }
    pub fn event(&self, name: &str, url: Option<&http::Uri>, fields: Value) {
        let mut event = fields.as_object().cloned().unwrap_or_default();
        event.insert("event".into(), json!(name));
        event.insert(
            "mono_ms".into(),
            json!(self.zero.elapsed().as_secs_f64() * 1000.),
        );
        if let Some(url) = url {
            static IDS: std::sync::LazyLock<regex::Regex> =
                std::sync::LazyLock::new(|| regex::Regex::new("[0-9]{6,}").unwrap());
            event.insert("url_id".into(), json!(hash(url.to_string().as_bytes())));
            event.insert("host".into(), json!(url.host()));
            event.insert(
                "path".into(),
                json!(if url.path().starts_with("/assets") {
                    url.path().to_owned()
                } else {
                    IDS.replace_all(url.path(), ":id").into_owned()
                }),
            );
            event.insert(
                "query_hash".into(),
                json!(url.query().map(|q| hash(q.as_bytes()))),
            );
        }
        if let Some(sender) = self.sender.lock().unwrap().as_ref() {
            if sender
                .try_send(serde_json::to_vec(&event).unwrap())
                .is_err()
            {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    pub async fn close(&self) {
        self.sender.lock().unwrap().take();
        let worker = self.worker.lock().unwrap().take();
        if let Some(worker) = worker {
            let _ = tokio::task::spawn_blocking(move || worker.join()).await;
        }
    }
}
impl Drop for Trace {
    fn drop(&mut self) {
        self.sender.get_mut().unwrap().take();
        if let Some(worker) = self.worker.get_mut().unwrap().take() {
            let _ = worker.join();
        }
    }
}
