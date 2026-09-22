use crate::{
    hash,
    memory::{Admission, MemoryCache},
    network::{http_headers, text_headers, Network, Reply, Request, Result},
    storage::{variant, Entry},
};
use bytes::Bytes;
use http_body_util::BodyExt;
use http::{Method, StatusCode};
use std::{
    collections::HashMap,
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::{watch, Notify, Semaphore};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Default)]
pub struct Counters {
    pub preloaded: AtomicU64,
    pub requests: AtomicU64,
    pub hits: AtomicU64,
    pub memory_hits: AtomicU64,
    pub failures: AtomicU64,
    pub disk_reads: AtomicU64,
    pub storage_failures: AtomicU64,
}
struct Storage {
    directory: PathBuf,
    identity: crate::directory::Identity,
    memory: MemoryCache,
}
struct Pending {
    result: watch::Sender<Option<Result<Arc<Reply>>>>,
    demand: AtomicBool,
    promoted: Notify,
}
pub struct Cache {
    residents_started: AtomicBool,
    network: Network,
    storage: Arc<Mutex<Storage>>,
    interval: i64,
    pending: Mutex<HashMap<String, Arc<Pending>>>,
    disk: Arc<Semaphore>,
    demand: Arc<Semaphore>,
    background: Arc<Semaphore>,
    api: Arc<Semaphore>,
    pub trace: Option<Arc<crate::trace::Trace>>,
    pub counters: Arc<Counters>,
    pub cancel: CancellationToken,
    tasks: TaskTracker,
}
pub struct CachedReply {
    pub reply: Arc<Reply>,
    pub state: &'static str,
    pub stream: Option<crate::network::Body>,
}
fn hold_stream(reply: &Reply, permit: tokio::sync::OwnedSemaphorePermit, counters: Arc<Counters>) {
    if let Some(stream) = &reply.stream {
        let mut stream = stream.0.lock().unwrap();
        if let Some(body) = stream.take() {
            *stream = Some(body.map_frame(move |frame| { let _ = &permit; frame })
                .map_err(move |error| { counters.failures.fetch_add(1, Ordering::Relaxed); error })
                .boxed_unsync());
        }
    }
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
fn entry(reply: &Reply, request: &Request) -> Option<Entry> {
    let headers = text_headers(&reply.headers)?;
    Some(Entry {
        checked: now(),
        variant: variant(&headers, &text_headers(&request.headers)?),
        headers,
        body: reply.body.to_vec(),
    })
}
fn restored(entry: &Arc<Entry>) -> Result<Arc<Reply>> {
    Ok(Arc::new(Reply {
        status: StatusCode::OK,
        headers: http_headers(&entry.headers)?,
        protocol: http::Version::HTTP_11,
        body: Bytes::from_owner(SharedBody(entry.clone())),
        stream: None,
    }))
}
fn conditional(reply: Arc<Reply>, request: &Request) -> Arc<Reply> {
    if reply.status == StatusCode::OK
        && text_headers(&reply.headers).is_some_and(|headers| {
            Entry {
                checked: 0,
                headers,
                variant: String::new(),
                body: vec![],
            }
            .not_modified(&text_headers(&request.headers).unwrap_or_default())
        })
    {
        let mut headers = reply.headers.clone();
        headers.remove("content-type");
        headers.remove("content-encoding");
        headers.remove("content-length");
        Arc::new(Reply {
            status: StatusCode::NOT_MODIFIED,
            headers,
            body: Bytes::new(),
            protocol: reply.protocol,
            stream: None,
        })
    } else {
        reply
    }
}
fn storable(reply: &Reply) -> bool {
    if reply.stream.is_some() || reply.headers.contains_key("set-cookie") {
        return false;
    }
    for name in ["cache-control", "pragma"] {
        for value in reply.headers.get_all(name) {
            let Ok(value) = value.to_str() else {
                return false;
            };
            if value.split([',', ';']).any(|part| {
                matches!(
                    part.split('=')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase()
                        .as_str(),
                    "no-cache" | "no-store" | "private" | "must-revalidate" | "proxy-revalidate"
                )
            }) {
                return false;
            }
        }
    }
    !reply.headers.get_all("vary").iter().any(|v| {
        v.to_str()
            .map_or(true, |s| s.split(',').any(|p| p.trim() == "*"))
    })
}
impl Cache {
    pub fn new(
        directory: PathBuf,
        memory_bytes: u64,
        interval_seconds: u32,
        network: Network,
    ) -> Result<Arc<Self>> {
        if !(1..=604800).contains(&interval_seconds) {
            return Err("invalid check interval".into());
        }
        std::fs::create_dir_all(&directory).map_err(|_| "cache_storage")?;
        Ok(Arc::new(Self {
            residents_started: AtomicBool::new(false),
            trace: network.trace.clone(),
            network,
            storage: Arc::new(Mutex::new(Storage {
                identity: crate::directory::Identity::open(&directory).map_err(|_| "cache_storage")?,
                directory,
                memory: MemoryCache::new(memory_bytes),
            })),
            interval: interval_seconds as i64 * 1000,
            pending: Mutex::new(HashMap::new()),
            disk: Arc::new(Semaphore::new(2)),
            demand: Arc::new(Semaphore::new(16)),
            background: Arc::new(Semaphore::new(2)),
            api: Arc::new(Semaphore::new(32)),
            counters: Arc::new(Counters::default()),
            cancel: CancellationToken::new(),
            tasks: TaskTracker::new(),
        }))
    }
    async fn stored(
        &self,
        key: String,
        request: Request,
        demand: bool,
    ) -> Result<Option<Arc<Entry>>> {
        let permit = self
            .disk
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| "stopped")?;
        let storage = self.storage.clone();
        let counters = self.counters.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut s = storage.lock().map_err(|_| "cache_storage")?;
            let headers = text_headers(&request.headers).ok_or("invalid request headers")?;
            if let Some(e) = s.memory.get(&key, false) {
                if !e.matches(&headers) {
                    return Ok(None);
                }
                if demand {
                    s.memory.get(&key, true);
                    counters.memory_hits.fetch_add(1, Ordering::Relaxed);
                }
                return Ok(Some(e));
            }
            counters.disk_reads.fetch_add(1, Ordering::Relaxed);
            if s.identity.check().is_err() { return Ok(None); }
            let path = s.directory.join(format!("{key}.gfc"));
            let read =
                std::fs::File::open(path).and_then(|f| Entry::read(std::io::BufReader::new(f)));
            let Ok(e) = read else {
                return Ok(None);
            };
            let e = Arc::new(e);
            if !e.matches(&headers) { return Ok(None); }
            if demand { s.memory.put(key, e.clone(), Admission::Demand); }
            Ok(Some(e))
        })
        .await
        .map_err(|_| "cache_storage")?
    }
    async fn store(&self, key: String, value: Option<Entry>, admission: Admission) -> Result<()> {
        let permit = self
            .disk
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| "stopped")?;
        let storage = self.storage.clone();
        let cancel = self.cancel.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut s = storage.lock().map_err(|_| "cache_storage")?;
            if cancel.is_cancelled() {
                return Err("stopped".into());
            }
            s.identity.check().map_err(|e| e.to_string())?;
            let path = s.directory.join(format!("{key}.gfc"));
            if let Some(e) = value {
                let mut file =
                    tempfile::NamedTempFile::new_in(&s.directory).map_err(|_| "cache_storage")?;
                e.write(file.as_file_mut()).map_err(|_| "cache_storage")?;
                file.flush().map_err(|_| "cache_storage")?;
                file.persist(path).map_err(|_| "cache_storage")?;
                s.memory.put(key, Arc::new(e), admission);
            } else {
                s.memory.remove(&key);
                match std::fs::remove_file(path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err("cache_storage".into()),
                }
            }
            Ok(())
        })
        .await
        .map_err(|_| "cache_storage")?
    }
    async fn fetch(&self, request: &Request, asset: bool, ignore_not_found: bool) -> Result<Arc<Reply>> {
        if asset { self.counters.requests.fetch_add(1, Ordering::Relaxed); }
        let reply = self.network.fetch(request, asset).await.map(Arc::new);
        if reply.as_ref().map_or(true, |r| {
            (r.status.is_client_error() || r.status.is_server_error())
                && !(ignore_not_found && r.status == StatusCode::NOT_FOUND)
        }) {
            self.counters.failures.fetch_add(1, Ordering::Relaxed);
        }
        reply
    }
    pub async fn forward(&self, request: http::Request<crate::network::Body>)
        -> Result<(http::Response<crate::network::Body>, tokio::sync::OwnedSemaphorePermit)> {
        let permit = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => return Err("stopped".into()),
            permit = self.api.clone().acquire_owned() => permit.map_err(|_| "stopped")?,
        };
        let result = tokio::select! {
            _ = self.cancel.cancelled() => Err("stopped".into()),
            result = self.network.forward(request) => result,
        };
        if result.as_ref().map_or(true, |r| r.status().is_client_error() || r.status().is_server_error()) {
            self.counters.failures.fetch_add(1, Ordering::Relaxed);
        }
        result.map(|reply| (reply, permit))
    }
    pub async fn pass(&self, request: &Request) -> Result<Arc<Reply>> {
        tokio::select! { _=self.cancel.cancelled()=>Err("stopped".into()),result=async {let _permit=self.api.acquire().await.map_err(|_|"stopped")?;self.fetch(request,false,false).await}=>result }
    }
    async fn delivery(&self, mut reply: Arc<Reply>, request: &Request, state: &'static str, background: bool) -> Result<CachedReply> {
        let mut stream = None;
        if !background && reply.stream.is_some() {
            stream = reply.stream.as_ref().unwrap().0.lock().unwrap().take();
            if stream.is_none() {
                // Other coalesced clients need their own non-cacheable stream; never replay failures.
                let permit = tokio::select! {
                    _ = self.cancel.cancelled() => return Err("stopped".into()),
                    permit = self.demand.clone().acquire_owned() => permit.map_err(|_| "stopped")?,
                };
                reply = tokio::select! {
                    _ = self.cancel.cancelled() => return Err("stopped".into()),
                    result = self.fetch(request, true, false) => result?,
                };
                hold_stream(&reply, permit, self.counters.clone());
                stream = reply.stream.as_ref().and_then(|body| body.0.lock().unwrap().take());
            }
        }
        Ok(CachedReply { reply, state, stream })
    }
    pub async fn get(self: &Arc<Self>, request: Request, background: bool) -> Result<CachedReply> {
        if self.cancel.is_cancelled() {
            return Err("stopped".into());
        }
        let bypass = request.method != Method::GET
            || text_headers(&request.headers).is_none()
            || [
                "cookie",
                "authorization",
                "range",
                "if-range",
                "if-match",
                "if-unmodified-since",
                "cache-control",
                "pragma",
            ]
            .iter()
            .any(|n| request.headers.contains_key(*n));
        if bypass {
            let pool = if background {
                &self.background
            } else {
                &self.demand
            };
            return tokio::select! {
                _ = self.cancel.cancelled() => Err("stopped".into()),
                result = async {
                    let permit = pool.clone().acquire_owned().await.map_err(|_| "stopped")?;
                    let reply = self.fetch(&request, true, background).await?;
                    hold_stream(&reply, permit, self.counters.clone());
                    self.delivery(reply, &request, "BYPASS", background).await
                } => result,
            };
        }
        let mut clean = request.clone();
        clean.headers.remove("if-none-match");
        clean.headers.remove("if-modified-since");
        let key = hash(clean.url.to_string().as_bytes());
        if let Some(cached) = self.stored(key.clone(), clean.clone(), !background).await? {
            if !background { self.counters.hits.fetch_add(1, Ordering::Relaxed); }
            if now() - cached.checked >= self.interval {
                self.refresh(key, clean, true);
            }
            return Ok(CachedReply {
                reply: conditional(restored(&cached)?, &request),
                state: "HIT",
                stream: None,
            });
        }
        let mut receiver = self.refresh(key, clean, background);
        loop {
            let result = receiver.borrow_and_update().clone();
            if let Some(result) = result {
                return self.delivery(conditional(result?, &request), &request, "MISS", background).await;
            }
            tokio::select! { _=self.cancel.cancelled()=>return Err("stopped".into()),changed=receiver.changed()=>if changed.is_err(){return Err("stopped".into());} }
        }
    }
    fn refresh(
        self: &Arc<Self>,
        key: String,
        request: Request,
        background: bool,
    ) -> watch::Receiver<Option<Result<Arc<Reply>>>> {
        let mut headers: Vec<_> = request
            .headers
            .iter()
            .map(|(n, v)| format!("{}:{}", n, v.to_str().unwrap_or("")))
            .collect();
        headers.sort();
        let identity = hash(format!("{}\n{}", request.url, headers.join("\n")).as_bytes());
        let mut pending = self.pending.lock().unwrap();
        if let Some(existing) = pending.get(&identity) {
            if !background {
                existing.demand.store(true, Ordering::Release);
                existing.promoted.notify_one();
            }
            return existing.result.subscribe();
        }
        let (sender, receiver) = watch::channel(None);
        let task = Arc::new(Pending {
            result: sender,
            demand: AtomicBool::new(!background),
            promoted: Notify::new(),
        });
        pending.insert(identity.clone(), task.clone());
        drop(pending);
        let cache = self.clone();
        self.tasks.spawn(async move {
            let result=tokio::select!{_=cache.cancel.cancelled()=>Err("stopped".into()),result=cache.download(&key,&request,&task)=>result};
            task.result.send_replace(Some(result));cache.pending.lock().unwrap().remove(&identity);
        });
        receiver
    }
    async fn download(
        &self,
        key: &str,
        request: &Request,
        pending: &Pending,
    ) -> Result<Arc<Reply>> {
        let permit = loop {
            if pending.demand.load(Ordering::Acquire) {
                break self.demand.clone().acquire_owned().await.map_err(|_| "stopped")?;
            }
            tokio::select! { biased; _=pending.promoted.notified()=>continue,permit=self.background.clone().acquire_owned()=>break permit.map_err(|_|"stopped")? }
        };
        let old = self.stored(key.into(), request.clone(), false).await?;
        if let Some(e) = &old {
            if now() - e.checked < self.interval {
                return restored(e);
            }
            let mut checked = (**e).clone();
            checked.checked = now();
            self.persist(key, Some(checked), request, Admission::Refresh).await;
        }
        let mut outgoing = request.clone();
        if let Some(e) = &old {
            let headers = http_headers(&e.headers)?;
            for name in ["etag", "last-modified"] {
                if let Some(value) = headers.get(name) {
                    outgoing.headers.insert(
                        if name == "etag" {
                            "if-none-match"
                        } else {
                            "if-modified-since"
                        },
                        value.clone(),
                    );
                    break;
                }
            }
        }
        let new_resource = old.is_none();
        let mut reply = self.fetch(&outgoing, true, true).await?;
        if matches!(reply.status, StatusCode::NOT_FOUND | StatusCode::GONE) && !new_resource {
            self.persist(key, None, request, Admission::Refresh).await;
        }
        // A browser waiting on this download, or validation of existing cache, is not speculative preload.
        if reply.status == StatusCode::NOT_FOUND && (!new_resource || pending.demand.load(Ordering::Acquire)) {
            self.counters.failures.fetch_add(1, Ordering::Relaxed);
        }
        if reply.status == StatusCode::NOT_MODIFIED {
            if let Some(e) = old {
                let mut headers = http_headers(&e.headers)?;
                for name in reply
                    .headers
                    .keys()
                    .filter(|n| n.as_str() != "content-encoding")
                {
                    headers.remove(name);
                    for value in reply.headers.get_all(name) {
                        headers.append(name, value.clone());
                    }
                }
                reply = Arc::new(Reply {
                    status: StatusCode::OK,
                    headers,
                    protocol: reply.protocol,
                    body: Bytes::from_owner(SharedBody(e.clone())),
                    stream: None,
                });
            }
        }
        if reply.status == StatusCode::OK {
            let stored = storable(&reply);
            let saved = self.persist(
                key,
                if stored {
                    entry(&reply, request)
                } else {
                    None
                },
                request,
                if pending.demand.load(Ordering::Acquire) { Admission::Demand }
                else if new_resource { Admission::Preload } else { Admission::Refresh },
            )
            .await;
            if saved && stored && new_resource && !pending.demand.load(Ordering::Acquire) {
                self.counters.preloaded.fetch_add(1, Ordering::Relaxed);
            }
        }
        hold_stream(&reply, permit, self.counters.clone());
        Ok(reply)
    }
    async fn persist(&self, key: &str, value: Option<Entry>, request: &Request, admission: Admission) -> bool {
        if let Err(error) = self.store(key.into(), value, admission).await {
            self.counters
                .storage_failures
                .fetch_add(1, Ordering::Relaxed);
            if let Some(trace) = &self.trace {
                trace.event(
                    "CACHE_WRITE_FAILED",
                    Some(&request.url),
                    serde_json::json!({"error":error}),
                );
            }
            return false;
        }
        true
    }
    pub fn pending_downloads(&self) -> usize {
        self.pending.lock().unwrap().len()
    }
    pub fn memory_bytes(&self) -> u64 {
        self.storage.lock().unwrap().memory.bytes()
    }
    pub fn restore_memory(self: &Arc<Self>) {
        if self.residents_started.swap(true, Ordering::Relaxed) { return; }
        let cache = self.clone();
        self.tasks.spawn(async move {
            // One background disk reader, using the same lock as normal cache reads.
            let worker = cache.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let directory = {
                    let storage = worker.storage.lock().unwrap();
                    if storage.identity.check().is_err() { return; }
                    storage.directory.clone()
                };
                let keys: Vec<String> = std::fs::read(directory.join("memory-resident.json"))
                    .ok().and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default();
                for key in keys {
                    if worker.cancel.is_cancelled() { break; }
                    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) { continue; }
                    let mut storage = worker.storage.lock().unwrap();
                    if storage.memory.get(&key, false).is_some() { continue; }
                    if storage.memory.restore_remaining() == 0 { break; }
                    if storage.identity.check().is_err() { break; }
                    let entry = std::fs::File::open(directory.join(format!("{key}.gfc")))
                        .and_then(|f| Entry::read(std::io::BufReader::new(f)));
                    if let Ok(entry) = entry {
                        storage.memory.restore(key, Arc::new(entry));
                    }
                    drop(storage);
                    std::thread::yield_now();
                }
            }).await;
            loop {
                tokio::select! {
                    _ = cache.cancel.cancelled() => break,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                        cache.save_residents().await;
                    }
                }
            }
        });
    }
    async fn save_residents(&self) {
        let storage = self.storage.clone();
        let result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            let (directory, keys) = {
                let mut s = storage.lock().unwrap();
                s.identity.check()?;
                s.memory.expire_idle();
                (s.directory.clone(), s.memory.resident_keys())
            };
            let mut file = tempfile::NamedTempFile::new_in(&directory)?;
            serde_json::to_writer(file.as_file_mut(), &keys)?;
            file.flush()?;
            file.persist(directory.join("memory-resident.json"))?;
            Ok(())
        }).await;
        if !matches!(result, Ok(Ok(()))) {
            self.counters.storage_failures.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub async fn close(&self) {
        self.cancel.cancel();
        self.tasks.close();
        self.tasks.wait().await;
        if self.residents_started.swap(false, Ordering::Relaxed) {
            self.save_residents().await;
        }
        self.storage.lock().unwrap().memory.close();
    }
}

struct SharedBody(Arc<Entry>);
impl AsRef<[u8]> for SharedBody {
    fn as_ref(&self) -> &[u8] {
        &self.0.body
    }
}
