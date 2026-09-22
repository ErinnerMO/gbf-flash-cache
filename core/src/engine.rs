use crate::{
    cache::{Cache, CachedReply},
    network::{Reply, Request, Result},
    refs::ResourceRefs,
    scheduler::{Scheduler, CAPACITY},
};
use http::{HeaderMap, Method, StatusCode};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};
use tokio::sync::{mpsc, oneshot};
use url::Url;

enum Event {
    Demand(String, f64),
    Response(Box<Request>, Option<Arc<Reply>>, bool, f64),
    Stop,
}
pub struct Engine {
    pub cache: Arc<Cache>,
    sender: mpsc::Sender<Event>,
    zero: Instant,
    done: tokio::sync::Mutex<Option<oneshot::Receiver<()>>>,
    pub dropped: Arc<AtomicU64>,
    origin: Url,
}
impl Engine {
    pub fn new(cache: Arc<Cache>, origin: Url) -> Result<Arc<Self>> {
        let mut parser =
            ResourceRefs::new(origin.as_str()).map_err(|_| "invalid resource origin")?;
        let (sender, mut receiver) = mpsc::channel(CAPACITY);
        let (done, finished) = oneshot::channel();
        let zero = Instant::now();
        let engine = Arc::new(Self {
            cache: cache.clone(),
            sender: sender.clone(),
            zero,
            done: tokio::sync::Mutex::new(Some(finished)),
            dropped: Arc::new(AtomicU64::new(0)),
            origin,
        });
        let runtime = tokio::runtime::Handle::current();
        let weak_sender = sender.downgrade();
        tokio::task::spawn_blocking(move || {
            let mut scheduler = Scheduler::default();
            let mut requests = HashMap::new();
            let mut active = 0;
            while let Some(event) = receiver.blocking_recv() {
                if cache.cancel.is_cancelled() {
                    break;
                }
                match event {
                    Event::Stop => break,
                    Event::Demand(url, at) => scheduler.demand(&url, at),
                    Event::Response(request, reply, background, at) => {
                        let identity = request.url.to_string();
                        let url = identity.as_str();
                        if background {
                            active -= 1;
                            scheduler
                                .completion(url, reply.as_ref().map_or(0, |r| r.status.as_u16()));
                        } else {
                            scheduler.response(
                                url,
                                reply.as_ref().is_some_and(|r| r.status == StatusCode::OK)
                                    && parser.asset(url),
                            );
                        }
                        if let Some(reply) = reply.filter(|r| r.status == StatusCode::OK && r.stream.is_none()) {
                            let get = |name: &str| {
                                reply
                                    .headers
                                    .get(name)
                                    .and_then(|v| v.to_str().ok())
                                    .unwrap_or("")
                            };
                            let refs = parser.parse(
                                url,
                                get("content-type"),
                                get("content-encoding"),
                                &reply.body,
                            );
                            for target in refs.keys() {
                                if requests.len() >= CAPACITY {
                                    break;
                                }
                                requests.entry(target.clone()).or_insert_with(|| {
                                    let mut headers = HeaderMap::new();
                                    for name in [
                                        "accept-encoding",
                                        "user-agent",
                                        "referer",
                                        "origin",
                                        "accept",
                                        "accept-language",
                                    ] {
                                        if let Some(value) = request.headers.get(name) {
                                            headers.insert(name, value.clone());
                                        }
                                    }
                                    Request {
                                        method: Method::GET,
                                        url: target.parse().unwrap(),
                                        headers,
                                        body: Default::default(),
                                    }
                                });
                            }
                            if let Some(log) = &cache.trace {
                                log.event("PARSE",Some(&request.url),serde_json::json!({"refs":refs.len(),"result":parser.reason,"background":background,"content_type":get("content-type"),"content_encoding":get("content-encoding"),"body_bytes":reply.body.len(),"observed_origins":parser.observed_origins}));
                            }
                            scheduler.discover(url, &refs, at);
                        }
                    }
                }
                // Drain queued browser events before assigning newly freed download slots.
                if !receiver.is_empty() {
                    continue;
                }
                while active < 2 && !cache.cancel.is_cancelled() {
                    let Some(job) = scheduler.take(zero.elapsed().as_secs_f64()) else {
                        break;
                    };
                    let Some(request) = requests.get(&job.url).cloned() else {
                        continue;
                    };
                    if let Some(log) = &cache.trace {
                        log.event("PRELOAD_START",Some(&request.url),serde_json::json!({"sequence":job.sequence,"weak":job.speculative,"sources":job.sources.len()}));
                    }
                    let Some(sender) = weak_sender.upgrade() else {
                        break;
                    };
                    active += 1;
                    let cache = cache.clone();
                    runtime.spawn(async move {
                        let reply = cache.get(request.clone(), true).await.ok().map(|r| r.reply);
                        let event = Event::Response(
                            Box::new(request),
                            reply,
                            true,
                            zero.elapsed().as_secs_f64(),
                        );
                        tokio::select! {_=cache.cancel.cancelled()=>{},_=sender.send(event)=>{}}
                    });
                }
            }
            let _ = done.send(());
        });
        Ok(engine)
    }
    fn emit(&self, event: Event) {
        if self.sender.try_send(event).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub fn is_asset(&self, url: &http::Uri) -> bool {
        ResourceRefs::new(self.origin.as_str()).unwrap().asset(&url.to_string())
    }
    pub fn observe_request(&self, request: &Request) {
        if let Some(log) = &self.cache.trace {
            log.event(
                "REQUEST",
                Some(&request.url),
                serde_json::json!({"method":request.method.as_str()}),
            );
        }
        self.emit(Event::Demand(
            request.url.to_string(),
            self.zero.elapsed().as_secs_f64(),
        ));
    }
    pub fn observe_response(&self, request: Request, reply: Option<Arc<Reply>>) {
        if let Some(log) = &self.cache.trace {
            log.event("RESPONSE", Some(&request.url), serde_json::json!({"status":reply.as_ref().map_or(502, |r| r.status.as_u16()),"result":if reply.is_some() {"PASS"} else {"ERROR"}}));
        }
        self.emit(Event::Response(Box::new(request), reply, false, self.zero.elapsed().as_secs_f64()));
    }
    pub async fn request(&self, request: Request) -> Result<CachedReply> {
        self.observe_request(&request);
        let asset = self.is_asset(&request.url);
        let result = if asset {
            self.cache.get(request.clone(), false).await
        } else {
            self.cache.pass(&request).await.map(|reply| CachedReply {
                reply,
                state: "PASS",
                stream: None,
            })
        };
        if let Some(log) = &self.cache.trace {
            log.event("RESPONSE",Some(&request.url),serde_json::json!({"status":result.as_ref().map_or(502,|r|r.reply.status.as_u16()),"result":result.as_ref().map_or("ERROR",|r|r.state)}));
        }
        self.emit(Event::Response(
            Box::new(request),
            result.as_ref().ok().map(|r| r.reply.clone()),
            false,
            self.zero.elapsed().as_secs_f64(),
        ));
        result
    }
    pub async fn close(&self) {
        self.cache.close().await;
        let _ = self.sender.send(Event::Stop).await;
        if let Some(done) = self.done.lock().await.take() {
            let _ = done.await;
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.cache.cancel.cancel();
    }
}
