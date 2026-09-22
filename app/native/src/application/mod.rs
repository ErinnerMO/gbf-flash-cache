mod connection;
use crate::platform;
pub use gbf_flash_cache_core::service::Fields;
use gbf_flash_cache_core::{
    certificates::Authority, network::Result, service::Service as CoreService,
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
const SETTINGS: [&str; 14] = [
    "port",
    "proxy",
    "protocol",
    "host",
    "proxyPort",
    "username",
    "memoryMiB",
    "keepLogs",
    "lan",
    "autoRun",
    "startHidden",
    "closeToTray",
    "themeMode",
    "acceleratorCompatibility",
];

/// Owns application settings and OS integration; the engine owns cache/network behavior.
pub struct Service {
    home: PathBuf,
    settings: Fields,
    core: CoreService,
    startup_warning: String,
    logs_cleanup_pending: bool,
}
impl Service {
    pub fn open(home: PathBuf) -> Result<Self> {
        Self::open_with_roots(home, platform::load_roots()?)
    }
    /// Trust roots are supplied by the platform host, never application settings.
    pub fn open_with_roots(home: PathBuf, roots: Vec<Vec<u8>>) -> Result<Self> {
        if roots.is_empty() {
            return Err("system trust store is empty".into());
        }
        let mut settings = match fs::read(home.join("settings.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| "设置文件损坏")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Fields::new(),
            Err(_) => return Err("无法读取设置".into()),
        };
        // Retired Android settings must not be returned or saved again.
        settings.remove("theme");
        settings.remove("capture");
        settings.remove("connection");
        let mut core = CoreService::open_with_directories(
            home.clone(),
            settings.get("cachePath").map(PathBuf::from),
            settings.get("logsPath").map(PathBuf::from),
        )?;
        core.set_roots(roots);
        let home = home.canonicalize().map_err(|_| "无法访问数据目录")?;
        let mut service = Self {
            core,
            home,
            settings,
            startup_warning: String::new(),
            logs_cleanup_pending: true,
        };
        service.prepare_logs();
        Ok(service)
    }
    // Retry deferred startup cleanup only while no session log can exist yet.
    fn prepare_logs(&mut self) {
        if !self.logs_cleanup_pending || self.core.ensure_logs_directory().is_err() {
            return;
        }
        match self.clean_logs() {
            Ok(()) => {
                self.logs_cleanup_pending = false;
                self.startup_warning.clear();
            }
            Err(error) => self.startup_warning = error,
        }
    }
    fn keep_logs(&self) -> bool {
        self.settings.get("keepLogs").is_some_and(|v| v == "true")
    }
    fn clean_logs(&mut self) -> Result<()> {
        if self.keep_logs() {
            return Ok(());
        }
        let root = self.core.logs_handle()?;
        for file in root.entries().map_err(|_| "无法清理日志")? {
            let file = file.map_err(|_| "无法清理日志")?;
            if file.file_type().map_err(|_| "无法清理日志")?.is_file()
                && Path::new(&file.file_name()).extension().is_some_and(|e| e == "jsonl")
            {
                root.remove_file(file.file_name()).map_err(|_| "无法清理日志")?;
            }
        }
        root.check().map_err(|e| e.to_string())
    }
    fn save(&mut self, args: &Fields) -> Result<()> {
        self.settings = Self::save_settings(&self.home, self.settings.clone(), args)?;
        Ok(())
    }
    fn save_settings(home: &Path, mut settings: Fields, args: &Fields) -> Result<Fields> {
        // A partial proxy toggle/protocol change must validate the merged credentials too.
        if ["proxy", "protocol", "username", "password"]
            .iter()
            .any(|key| args.contains_key(*key))
        {
            let mut candidate = settings.clone();
            candidate.extend(args.clone());
            if candidate.get("proxy").is_some_and(|v| v == "true")
                && !candidate
                    .get("protocol")
                    .is_some_and(|v| v.eq_ignore_ascii_case("socks4"))
                && !args.contains_key("password")
            {
                if let Some(secret) = settings.get("protectedPassword") {
                    let password = platform::unprotect_password(secret)
                        .ok()
                        .flatten()
                        .ok_or("上游代理密码无法解密，请重新填写并保存")?;
                    candidate.insert("password".into(), password);
                } else if settings.contains_key("androidProtectedPassword") {
                    // Android's host supplies plaintext after Keystore decryption.
                    return Err("请在连接设置中重新填写并保存上游代理密码".into());
                }
            }
            connection::validate_credentials(&candidate)?;
        }
        for key in SETTINGS {
            if let Some(value) = args.get(key) {
                settings.insert(key.into(), value.clone());
            }
        }
        #[cfg(target_os = "android")]
        if let Some(protected) = args.get("androidProtectedPassword") {
            if protected.is_empty() {
                settings.remove("androidProtectedPassword");
            } else {
                settings.insert("androidProtectedPassword".into(), protected.clone());
            }
        }
        if let Some(password) = args.get("password") {
            if let Some(protected) = platform::protect_password(password)? {
                if password.is_empty() {
                    settings.remove("protectedPassword");
                } else {
                    settings.insert("protectedPassword".into(), protected);
                }
            }
        }
        let mut temporary = tempfile::NamedTempFile::new_in(home).map_err(|_| "无法保存设置")?;
        temporary
            .write_all(&serde_json::to_vec(&settings).map_err(|_| "无法保存设置")?)
            .map_err(|_| "无法保存设置")?;
        temporary
            .persist(home.join("settings.json"))
            .map_err(|_| "无法保存设置")?;
        Ok(settings)
    }
    async fn ca_status(&mut self) -> Result<Fields> {
        let mut out = self.core.command("ca_status", Fields::new()).await?;
        out.insert("trusted".into(), "unknown".into());
        if let Ok(cert) = Authority::stored_certificate(&self.home.join("ca")) {
            match platform::trust(&cert, "status") {
                Ok(Some(value)) => {
                    out.insert("trusted".into(), value.to_string());
                }
                Ok(None) => {}
                Err(e) => {
                    out.insert("warning".into(), e);
                }
            }
        }
        Ok(out)
    }
    pub async fn command(&mut self, op: &str, args: Fields) -> Result<Fields> {
        if matches!(op, "init" | "status" | "start" | "export") {
            self.prepare_logs();
            if matches!(op, "start" | "export") && self.logs_cleanup_pending {
                return Err(self
                    .core
                    .directory_error("logs")
                    .unwrap_or(&self.startup_warning)
                    .to_owned());
            }
        }
        let mut out = Fields::new();
        match op {
            "init" => {
                out = self.settings.clone();
                if !self.startup_warning.is_empty() {
                    out.insert("startupWarning".into(), self.startup_warning.clone());
                }
                if let Some(enabled) = platform::startup(None)? {
                    out.insert("autoStart".into(), enabled.to_string());
                }
                if let Some(secret) = out.remove("protectedPassword") {
                    match platform::unprotect_password(&secret) {
                        Ok(Some(password)) => {
                            out.insert("password".into(), password);
                        }
                        _ => {
                            out.insert("password".into(), String::new());
                            out.insert(
                                "passwordWarning".into(),
                                "上游代理密码无法解密，请重新填写并保存".into(),
                            );
                        }
                    }
                }
                out.extend(self.core.command("init", Fields::new()).await?);
            }
            "settings" => self.save(&args)?,
            "start" => {
                out = self.core.command(op, connection::resolve(&args)?).await?;
                if let Err(error) = self.save(&args) {
                    self.core.command("stop", Fields::new()).await?;
                    return Err(error);
                }
            }
            "probe" => out = self.core.command(op, connection::resolve(&args)?).await?,
            "directory" => {
                let mut settings = self.settings.clone();
                let home = &self.home;
                let target = self.core.change_directory(&args, |target| {
                    settings.insert(
                        format!("{}Path", args["kind"]),
                        target.to_string_lossy().into(),
                    );
                    settings = Self::save_settings(home, settings.clone(), &args)?;
                    Ok(())
                })?;
                self.settings = settings;
                out.insert("path".into(), target.to_string_lossy().into());
            }
            "startup" => {
                let enabled = args.get("enabled").is_some_and(|v| v == "true");
                let actual =
                    platform::startup(Some(enabled))?.ok_or("当前平台不支持开机启动设置")?;
                out.insert("autoStart".into(), actual.to_string());
            }
            "ca_status" => out = self.ca_status().await?,
            "ca_install" | "ca_uninstall" => {
                if op == "ca_uninstall" && self.core.is_running() {
                    return Err("请先停止服务".into());
                }
                let directory = self.home.join("ca");
                let cert = if op == "ca_install" {
                    Authority::open(&directory)
                        .map_err(|_| "CA 无效或已过期，请重新生成")?
                        .certificate
                } else {
                    Authority::stored_certificate(&directory).map_err(|_| "无法读取 CA 证书")?
                };
                if let Some(expected) = args.get("fingerprint").filter(|s| !s.is_empty()) {
                    if *expected != gbf_flash_cache_core::hash(&cert) {
                        return Err("CA 证书已变化，请重新打开证书管理".into());
                    }
                }
                platform::trust(
                    &cert,
                    if op == "ca_install" {
                        "install"
                    } else {
                        "uninstall"
                    },
                )?;
                out = self.ca_status().await?;
            }
            "ca_regenerate" => {
                let old = Authority::stored_certificate(&self.home.join("ca")).ok();
                self.core.command(op, args).await?;
                out = self.ca_status().await?;
                if let Some(old) = old {
                    if let Err(e) = platform::remove_previous_trust(&old) {
                        out.insert("warning".into(), format!("新证书已生成；旧证书信任清理失败：{e}。旧证书保存在 ca/previous-ca.cer"));
                    }
                }
            }
            "export" => {
                if self.core.is_running() {
                    return Err("请先停止服务再导出日志".into());
                }
                let path = PathBuf::from(args.get("path").ok_or("请选择导出文件")?);
                let parent = path
                    .parent()
                    .ok_or("导出路径无效")?
                    .canonicalize()
                    .map_err(|_| "导出目录无效")?;
                if parent.starts_with(&self.home) {
                    return Err("请导出到应用数据目录以外".into());
                }
                let destination = parent.join(path.file_name().ok_or("导出文件名无效")?);
                let logs = self.core.logs_handle()?;
                export_logs(&logs, &destination)?;
                out.insert("path".into(), destination.to_string_lossy().into());
            }
            _ => out = self.core.command(op, args).await?,
        }
        if matches!(op, "init" | "status") && !self.startup_warning.is_empty() {
            let warning = out.entry("directoryWarning".into()).or_default();
            if !warning.is_empty() {
                warning.push_str("；");
            }
            warning.push_str(&self.startup_warning);
        }
        out.insert("running".into(), self.core.is_running().to_string());
        Ok(out)
    }
    pub async fn close(&mut self) -> Result<()> {
        self.core.close().await
    }
}
fn export_logs(root: &gbf_flash_cache_core::service::DirectoryHandle, destination: &Path) -> Result<()> {
    let mut temporary =
        tempfile::NamedTempFile::new_in(destination.parent().ok_or("导出路径无效")?)
            .map_err(|_| "无法导出日志")?;
    {
        let mut zip = zip::ZipWriter::new(temporary.as_file_mut());
        for file in root.entries().map_err(|_| "无法读取日志")? {
            let file = file.map_err(|_| "无法读取日志")?;
            if file.file_type().map_err(|_| "无法读取日志")?.is_file()
                && Path::new(&file.file_name()).extension().is_some_and(|e| e == "jsonl")
            {
                zip.start_file(
                    file.file_name().to_string_lossy(),
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Stored),
                )
                .map_err(|_| "无法导出日志")?;
                let mut input = root.open(file.file_name()).map_err(|_| "无法读取日志")?;
                std::io::copy(&mut input, &mut zip).map_err(|_| "无法导出日志")?;
            }
        }
        root.check().map_err(|e| e.to_string())?;
        zip.finish().map_err(|_| "无法导出日志")?;
    }
    temporary.persist(destination).map_err(|_| "无法导出日志")?;
    Ok(())
}
