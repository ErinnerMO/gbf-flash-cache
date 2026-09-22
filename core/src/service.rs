use crate::{
    cache::Cache,
    certificates::Authority,
    engine::Engine,
    gateway::Gateway,
    network::{Network, Result},
    trace::Trace,
    tunnel::Route,
    CDN,
};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc},
    time::{SystemTime, UNIX_EPOCH},
};
use url::Url;
pub use crate::directory::DirectoryHandle;
pub type Fields = BTreeMap<String, String>;
// Held even while stopped: clear, log cleanup and migration also need exclusive ownership.
struct Directory {
    path: PathBuf,
    _lock: Option<File>,
    _directory_lock: Option<File>,
    identity: Option<crate::directory::Identity>,
    error: Option<String>,
}
impl Directory {
    fn recoverable(path: PathBuf) -> Self {
        Self::open(&path).unwrap_or_else(|error| Self {
            path,
            _lock: None,
            _directory_lock: None,
            identity: None,
            error: Some(error),
        })
    }
    fn ensure(&mut self) -> Result<()> {
        if let Some(identity) = &self.identity {
            if let Err(error) = identity.check() {
                self.error = Some(error.to_string());
                return Err(error.to_string());
            }
        }
        if self.identity.is_some() { self.error = None; }
        if self._lock.is_none() {
            *self = Self::recoverable(self.path.clone());
        }
        self.error.clone().map_or(Ok(()), Err)
    }
    fn open(path: &Path) -> Result<Self> {
        if path.is_symlink() {
            return Err("目录不能是符号链接".into());
        }
        fs::create_dir_all(path).map_err(|_| "无法创建目录")?;
        let path = path.canonicalize().map_err(|_| "无法访问目录")?;
        if path.join(".gbf-flash-cache.lock").is_symlink() { return Err("目录锁不能是符号链接".into()); }
        let mut options = fs::OpenOptions::new();
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Keep the lock file itself non-replaceable while this owner lives.
            options.share_mode(0x00000003);
        }
        let lock = options
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.join(".gbf-flash-cache.lock"))
            .map_err(|_| "无法打开目录锁")?;
        lock.try_lock().map_err(|_| "目录已被另一个实例使用")?;
        let identity = crate::directory::Identity::open(&path).map_err(|e| e.to_string())?;
        let directory_lock = identity.lock_directory().map_err(|_| "目录已被另一个实例使用")?;
        if !identity.matches_lock(&lock).map_err(|e| e.to_string())? {
            return Err("目录锁已被替换，请重试".into());
        }
        identity.check().map_err(|e| e.to_string())?;
        Ok(Self {
            identity: Some(identity),
            path,
            _lock: Some(lock),
            _directory_lock: directory_lock,
            error: None,
        })
    }
    fn recover_changed(&mut self) {
        if self.identity.as_ref().is_some_and(|id| id.check().is_err()) {
            self._lock = None;
            self._directory_lock = None;
            self.identity = None;
            *self = Self::recoverable(self.path.clone());
        }
    }
    fn replacement(&self, path: &Path) -> Result<Option<Self>> {
        if path.is_symlink() {
            return Err("目录不能是符号链接".into());
        }
        if self._lock.is_some() && self.identity.as_ref().is_some_and(|id| id.check().is_ok()) && path.canonicalize().is_ok_and(|path| path == self.path) {
            return Ok(None);
        }
        Self::open(path).map(Some)
    }
}
/// Platform hosts own one service and serialize commands. No network control endpoint.
pub struct Service {
    home: PathBuf,
    _lock: File,
    cache_dir: Directory,
    logs_dir: Directory,
    roots: Vec<Vec<u8>>,
    gateway: Option<Arc<Gateway>>,
    cache: Option<Arc<Cache>>,
    trace: Option<Arc<Trace>>,
    counters: Fields,
}
impl Service {
    pub fn open(home: PathBuf) -> Result<Self> {
        Self::open_with_directories(home, None, None)
    }
    pub fn open_with_directories(
        home: PathBuf,
        cache: Option<PathBuf>,
        logs: Option<PathBuf>,
    ) -> Result<Self> {
        fs::create_dir_all(&home).map_err(|_| "无法创建数据目录")?;
        let home = home.canonicalize().map_err(|_| "无法访问数据目录")?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(home.join("app.lock"))
            .map_err(|_| "无法打开数据目录")?;
        lock.try_lock().map_err(|_| "数据目录已被另一个实例使用")?;
        let service = Self {
            cache_dir: Directory::recoverable(cache.unwrap_or_else(|| home.join("cache"))),
            logs_dir: Directory::recoverable(logs.unwrap_or_else(|| home.join("logs"))),
            roots: Vec::new(),
            home,
            _lock: lock,
            gateway: None,
            cache: None,
            trace: None,
            counters: Fields::new(),
        };
        Ok(service)
    }
    pub fn configure(&mut self, cache: PathBuf, logs: PathBuf, roots: Vec<Vec<u8>>) -> Result<()> {
        if self.gateway.is_some() {
            return Err("请先停止服务".into());
        }
        // Acquire both replacements before releasing either old directory.
        let cache = self.cache_dir.replacement(&cache)?;
        let logs = self.logs_dir.replacement(&logs)?;
        if let Some(cache) = cache {
            self.cache_dir = cache;
        }
        if let Some(logs) = logs {
            self.logs_dir = logs;
        }
        self.roots = roots;
        Ok(())
    }
    /// Keep both directories locked until the application has persisted its settings.
    pub fn change_directory(
        &mut self,
        args: &Fields,
        commit: impl FnOnce(&Path) -> Result<()>,
    ) -> Result<PathBuf> {
        if self.gateway.is_some() {
            return Err("请先停止服务".into());
        }
        let kind = args.get("kind").map(String::as_str).ok_or("目录类型缺失")?;
        if !["cache", "logs"].contains(&kind) {
            return Err("目录类型无效".into());
        }
        let base = PathBuf::from(args.get("path").ok_or("请选择目录")?);
        let base = base.canonicalize().map_err(|_| "无法访问所选目录")?;
        let target = base.join(format!("gbf-flash-cache-{kind}"));
        let current = if kind == "cache" {
            &self.cache_dir
        } else {
            &self.logs_dir
        };
        let replacement = current.replacement(&target)?;
        let target = target.canonicalize().map_err(|_| "无法访问所选目录")?;
        let destination_owner = replacement.as_ref().unwrap_or(current);
        let destination = destination_owner.identity.as_ref().ok_or("目标目录不可用")?
            .snapshot().map_err(|e| e.to_string())?;
        let marker = ".gbf-flash-cache";
        if !destination.open(marker).is_ok_and(|f| f.metadata().is_ok_and(|m| m.is_file())) {
            for entry in destination.entries().map_err(|_| "无法读取目录")? {
                if entry.map_err(|_| "无法读取目录")?.file_name() != ".gbf-flash-cache.lock" {
                    return Err("目标目录含有其它文件，请选择空目录".into());
                }
            }
            use std::io::Write;
            destination.create_new(marker).and_then(|mut f| f.write_all(kind.as_bytes()))
                .map_err(|_| "目录不可写")?;
        }
        let mut copied = Vec::new();
        let result = (|| {
            if kind == "cache"
                && args.get("migrate").is_some_and(|s| s == "true")
                && current.path != target
            {
                if let Some(error) = &current.error { return Err(error.clone()); }
                let source = current.identity.as_ref().ok_or("旧缓存目录不可用")?
                    .snapshot().map_err(|e| e.to_string())?;
                for file in source.entries().map_err(|_| "无法读取旧缓存")? {
                    let file = file.map_err(|_| "无法读取旧缓存")?;
                    let name = file.file_name();
                    if file.file_type().map_err(|_| "无法读取旧缓存")?.is_file()
                        && Path::new(&name).extension().is_some_and(|e| e == "gfc")
                    {
                        let mut input = source.open(&name).map_err(|e| e.to_string())?;
                        destination.copy_new(&name, &mut input, || source.check()).map_err(|e| {
                            if e.kind() == std::io::ErrorKind::AlreadyExists {
                                "目标目录已有缓存，请选择空目录迁移".to_owned()
                            } else { format!("无法迁移缓存：{e}") }
                        })?;
                        copied.push(name);
                        destination.check().map_err(|e| e.to_string())?;
                    }
                }
                source.check().map_err(|e| e.to_string())?;
            }
            destination.check().map_err(|e| e.to_string())?;
            commit(&target)
        })();
        if let Err(error) = result {
            for file in copied {
                destination.remove_file(file).map_err(|_| format!("{error}；无法清理本次迁移文件"))?;
            }
            return Err(error);
        }
        if let Some(directory) = replacement {
            if kind == "cache" {
                self.cache_dir = directory;
            } else {
                self.logs_dir = directory;
            }
        }
        Ok(target)
    }
    pub fn set_roots(&mut self, roots: Vec<Vec<u8>>) {
        self.roots = roots;
    }
    pub fn ensure_logs_directory(&mut self) -> Result<()> {
        self.logs_dir.ensure()
    }
    pub fn logs_handle(&mut self) -> Result<DirectoryHandle<'_>> {
        self.logs_dir.ensure()?;
        self.logs_dir.identity.as_ref().ok_or("日志目录不可用")?
            .snapshot().map_err(|e| e.to_string())
    }
    pub fn directory_error(&self, kind: &str) -> Option<&str> {
        if kind == "cache" {
            self.cache_dir.error.as_deref()
        } else {
            self.logs_dir.error.as_deref()
        }
    }
    pub fn is_running(&self) -> bool {
        self.gateway.is_some()
    }
    fn directory(&self, kind: &str) -> PathBuf {
        if kind == "cache" {
            self.cache_dir.path.clone()
        } else {
            self.logs_dir.path.clone()
        }
    }
    fn snapshot(&mut self) {
        if let Some(cache) = &self.cache {
            for (key, counter) in [
                ("preloaded", &cache.counters.preloaded),
                ("hits", &cache.counters.hits),
                ("requests", &cache.counters.requests),
                ("failures", &cache.counters.failures),
                ("memoryHits", &cache.counters.memory_hits),
                ("storageFailures", &cache.counters.storage_failures),
            ] {
                self.counters
                    .insert(key.into(), counter.load(Ordering::Relaxed).to_string());
            }
        }
    }
    async fn stop(&mut self) {
        self.snapshot();
        if let Some(gateway) = self.gateway.take() {
            gateway.close().await;
        }
        self.cache = None;
        if let Some(trace) = self.trace.take() {
            trace.event("STOP", None, serde_json::json!({}));
            trace.close().await;
        }
        self.cache_dir.recover_changed();
        self.logs_dir.recover_changed();
    }
    pub async fn command(&mut self, op: &str, args: Fields) -> Result<Fields> {
        let mut out = Fields::new();
        match op {
            "init" => {
                for kind in ["cache", "logs"] {
                    out.insert(
                        format!("{kind}Path"),
                        self.directory(kind).to_string_lossy().into(),
                    );
                }
                out.insert("home".into(), self.home.to_string_lossy().into());
                out.insert("version".into(), env!("CARGO_PKG_VERSION").into());
            }
            "directory" => {
                let target = self.change_directory(&args, |_| Ok(()))?;
                out.insert("path".into(), target.to_string_lossy().into());
            }
            "start" => {
                if self.gateway.is_some() {
                    return Err("服务已经启动".into());
                }
                self.cache_dir.ensure()?;
                self.logs_dir.ensure()?;
                let port = number(&args, "port", 8765)?;
                if port == 0 || port > 65535 {
                    return Err("监听端口须为 1–65535".into());
                }
                let memory = number(&args, "memoryMiB", 128)?
                    .checked_mul(1048576)
                    .ok_or("内存缓存容量过大")?;
                let upstream = proxy_address(&args, port as u16).await?;
                let route = Route::new(upstream.as_deref(), &self.roots)?;
                let ca = Authority::open(&self.home.join("ca")).map_err(|_| "无法读取或创建 CA")?;
                let logs = self.directory("logs");
                fs::create_dir_all(&logs).map_err(|_| "无法创建日志目录")?;
                let trace = Trace::open(&logs.join(format!(
                        "{}.jsonl",
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos()
                    )))
                .map_err(|_| "无法创建日志")?;
                trace.event("START",None,serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"strategy":"window_tiers"}));
                let network =
                    Network::new(upstream.as_deref(), &self.roots)?.with_trace(trace.clone());
                let cache = Cache::new(self.directory("cache"), memory, 3600, network)?;
                let engine = Engine::new(cache.clone(), Url::parse(CDN).unwrap())?;
                let origins: Vec<Url> = crate::CDN_HOSTS
                    .iter()
                    .map(|host| Url::parse(&format!("https://{host}")).unwrap())
                    .chain(
                        [
                            "https://game.granbluefantasy.jp",
                            "https://gbf.game.mbga.jp",
                        ]
                        .map(|s| Url::parse(s).unwrap()),
                    )
                    .collect();
                let gateway = match Gateway::start_with_lan(
                    port as u16,
                    engine.clone(),
                    &ca,
                    &origins,
                    route,
                    args.get("lan").is_some_and(|v| v == "true"),
                )
                .await
                {
                    Ok(g) => g,
                    Err(e) => {
                        engine.close().await;
                        trace.close().await;
                        return Err(e);
                    }
                };
                self.gateway = Some(gateway);
                self.cache = Some(cache.clone());
                self.trace = Some(trace);
                self.counters.clear();
                cache.restore_memory();
            }
            "stop" => self.stop().await,
            "clear" => {
                if self.gateway.is_some() {
                    return Err("请先停止服务".into());
                }
                self.cache_dir.ensure()?;
                let cache = self.cache_dir.identity.as_ref().ok_or("缓存目录不可用")?
                    .snapshot().map_err(|e| e.to_string())?;
                for file in cache.entries().map_err(|_| "无法清理缓存")? {
                    let file = file.map_err(|_| "无法清理缓存")?;
                    if file.file_type().map_err(|_| "无法清理缓存")?.is_file()
                        && Path::new(&file.file_name()).extension().is_some_and(|e| e == "gfc")
                    {
                        cache.remove_file(file.file_name()).map_err(|_| "无法清理缓存")?;
                    }
                }
                if let Err(error) = cache.remove_file("memory-resident.json") {
                    if error.kind() != std::io::ErrorKind::NotFound { return Err(error.to_string()); }
                }
                cache.check().map_err(|e| e.to_string())?;
                self.counters.clear();
            }
            "probe" => {
                let port = number(&args, "port", 8765)?;
                if port == 0 || port > 65535 {
                    return Err("监听端口须为 1–65535".into());
                }
                let upstream = proxy_address(&args, port as u16).await?;
                let network = Network::new(upstream.as_deref(), &self.roots)?;
                let request = crate::network::Request {
                    method: http::Method::GET,
                    url: "https://game.granbluefantasy.jp/".parse().unwrap(),
                    headers: http::HeaderMap::new(),
                    body: bytes::Bytes::new(),
                };
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(15),
                    network.fetch(&request, false),
                )
                .await
                .map_err(|_| "连接超时，请检查网络或上游代理")?
                .map_err(|e| match e.as_str() {
                    "timeout" => "连接超时，请检查网络或上游代理".to_owned(),
                    "upstream_tls" => "TLS 连接失败，请检查上游网络".to_owned(),
                    _ => format!("连接失败：{e}"),
                })?;
                if !(result.status.is_success() || result.status.is_redirection()) {
                    return Err(format!("服务器返回 HTTP {}", result.status.as_u16()));
                }
                out.insert("status".into(), result.status.as_u16().to_string());
            }
            "ca_status" => out = self.ca_status(),
            "ca_regenerate" => {
                if self.gateway.is_some() {
                    return Err("请先停止服务".into());
                }
                let directory = self.home.join("ca");
                let old = Authority::stored_certificate(&directory).ok();
                if args.get("fingerprint").map(String::as_str).unwrap_or("")
                    != old.as_ref().map(|c| crate::hash(c)).unwrap_or_default()
                {
                    return Err("CA 证书已变化，请重新打开证书管理".into());
                }
                if let Some(old) = &old {
                    fs::write(directory.join("previous-ca.cer"), old)
                        .map_err(|_| "无法保存旧 CA 证书")?;
                }
                Authority::regenerate(&directory).map_err(|_| "重新生成失败，现有 CA 保持不变")?;
                let _ = fs::remove_file(directory.join("gbf-flash-cache.cer"));
                out = self.ca_status();
            }
            "ca" => {
                let ca = Authority::open(&self.home.join("ca")).map_err(|_| "无法读取或创建 CA")?;
                let path = self.home.join("ca/gbf-flash-cache.cer");
                fs::write(&path, ca.certificate).map_err(|_| "无法导出 CA")?;
                out.insert("path".into(), path.to_string_lossy().into());
            }
            "status" => {
                let _ = self.cache_dir.ensure();
                let _ = self.logs_dir.ensure();
                self.snapshot();
                out.extend(self.counters.clone());
                let mut bytes = 0u64;
                let mut entries = 0u64;
                let cache = self.directory("cache");
                if self.cache_dir.error.is_none() && self.cache_dir._lock.is_some() && cache.exists() {
                    for file in fs::read_dir(cache).map_err(|_| "无法读取缓存大小")? {
                        let file = file.map_err(|_| "无法读取缓存大小")?;
                        if file.path().extension().is_some_and(|e| e == "gfc") {
                            if let Ok(meta) = file.metadata() {
                                if meta.is_file() {
                                    bytes = bytes.saturating_add(meta.len());
                                    entries += 1;
                                }
                            }
                        }
                    }
                }
                out.insert("diskBytes".into(), bytes.to_string());
                out.insert("diskEntries".into(), entries.to_string());
                out.insert(
                    "memoryBytes".into(),
                    self.cache
                        .as_ref()
                        .map_or(0, |c| c.memory_bytes())
                        .to_string(),
                );
                if let Some(trace) = &self.trace {
                    out.insert(
                        "logDropped".into(),
                        trace.dropped.load(Ordering::Relaxed).to_string(),
                    );
                    out.insert(
                        "logFailed".into(),
                        trace.failed.load(Ordering::Relaxed).to_string(),
                    );
                }
            }
            _ => return Err("未知命令".into()),
        }
        let warning = [("cache", "缓存"), ("logs", "日志")]
            .iter()
            .filter_map(|(kind, label)| {
                self.directory_error(kind)
                    .map(|error| format!("{label}目录不可用：{error}。请在设置中重新选择目录。"))
            })
            .collect::<Vec<_>>()
            .join("\n");
        out.insert("directoryWarning".into(), warning);
        out.insert("running".into(), self.gateway.is_some().to_string());
        if op == "status" {
            if let Some(log) = &self.trace { log.event("STATUS", None, serde_json::json!(out)); }
        }
        Ok(out)
    }
    fn ca_status(&self) -> Fields {
        let mut result = Fields::new();
        let dir = self.home.join("ca");
        result.insert("state".into(), "missing".into());
        if !dir.join("authority.json").exists() {
            return result;
        }
        result.insert("state".into(), "invalid".into());
        if let Ok(cert) = Authority::stored_certificate(&dir) {
            result.insert("fingerprint".into(), crate::hash(&cert));
            if let Ok((_, parsed)) = x509_parser::parse_x509_certificate(&cert) {
                result.insert(
                    "notBefore".into(),
                    parsed
                        .validity()
                        .not_before
                        .to_datetime()
                        .date()
                        .to_string(),
                );
                result.insert(
                    "notAfter".into(),
                    parsed.validity().not_after.to_datetime().date().to_string(),
                );
            }
            if Authority::open(&dir).is_ok() {
                result.insert("state".into(), "valid".into());
            }
        }
        result
    }
    pub async fn close(&mut self) -> Result<()> {
        self.stop().await;
        Ok(())
    }
}
fn number(args: &Fields, key: &str, default: u64) -> Result<u64> {
    args.get(key)
        .map(|s| {
            s.parse::<u64>()
                .map_err(|_| format!("{key} 必须为非负整数"))
        })
        .unwrap_or(Ok(default))
}
// The application supplies a resolved upstream URL, not UI connection settings.
async fn proxy_address(args: &Fields, listen: u16) -> Result<Option<String>> {
    let Some(address) = args.get("upstream") else {
        return Ok(None);
    };
    let url = Url::parse(address).map_err(|_| "invalid proxy address")?;
    let host = url.host_str().ok_or("invalid proxy address")?;
    let port = url.port_or_known_default().ok_or("invalid proxy port")?;
    let listen = std::net::SocketAddr::from((
        if args.get("lan").is_some_and(|v| v == "true") {
            [0, 0, 0, 0]
        } else {
            [127, 0, 0, 1]
        },
        listen,
    ));
    crate::tunnel::check_loop(host, port, listen).await?;
    Ok(Some(address.clone()))
}

#[cfg(test)]
mod directory_tests {
    use super::*;
    #[test]
    fn replacing_lock_stops_snapshot_and_cannot_transfer_ownership() {
        let temp = tempfile::tempdir().unwrap();
        let owner = Directory::open(temp.path()).unwrap();
        let snapshot = owner.identity.as_ref().unwrap().snapshot().unwrap();
        fs::write(temp.path().join("first.gfc"), b"first").unwrap();
        fs::write(temp.path().join("remaining.gfc"), b"remaining").unwrap();
        snapshot.remove_file("first.gfc").unwrap();
        let lock = temp.path().join(".gbf-flash-cache.lock");
        if let Err(error) = fs::rename(&lock, temp.path().join("old-lock")) {
            assert!(cfg!(windows), "{error}");
            assert!(Directory::open(temp.path()).is_err());
            return;
        }
        let replacement = File::create(&lock).unwrap();
        replacement.try_lock().unwrap();
        assert!(snapshot.remove_file("remaining.gfc").is_err());
        assert!(snapshot.open("remaining.gfc").is_err());
        assert!(snapshot.create_new("new.gfc").is_err());
        assert!(temp.path().join("remaining.gfc").exists());
        drop(replacement);
        // Even without a held replacement-file lock, the directory inode is owned.
        assert!(Directory::open(temp.path()).is_err());
        drop(snapshot);
        drop(owner);
        assert!(Directory::open(temp.path()).is_ok());
    }
}
