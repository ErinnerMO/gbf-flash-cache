//! Current-user Windows integration. Never modifies system proxy settings.
use gbf_flash_cache_core::network::Result;
use windows_sys::Win32::{Security::Cryptography::*, System::Registry::*};

pub fn trust(cert: &[u8], action: &str) -> Result<bool> {
    unsafe {
        let name: Vec<u16> = "ROOT\0".encode_utf16().collect();
        let mut installed = false;
        for scope in [CERT_SYSTEM_STORE_CURRENT_USER, CERT_SYSTEM_STORE_LOCAL_MACHINE] {
            let readonly = action == "status" || scope == CERT_SYSTEM_STORE_LOCAL_MACHINE;
            let store = CertOpenStore(CERT_STORE_PROV_SYSTEM_W, 0, 0,
                scope | CERT_STORE_OPEN_EXISTING_FLAG | if readonly { CERT_STORE_READONLY_FLAG } else { 0 }, name.as_ptr().cast());
            if store.is_null() { return Err("无法读取 Windows 证书存储区".into()); }
            let mut context = std::ptr::null();
            let mut failed = false;
            loop {
                context = CertEnumCertificatesInStore(store, context);
                if context.is_null() { break; }
                let bytes = std::slice::from_raw_parts((*context).pbCertEncoded, (*context).cbCertEncoded as usize);
                if bytes == cert {
                    if action == "uninstall" && !readonly {
                        // Delete consumes the context. Enumerate again after deletion.
                        failed = CertDeleteCertificateFromStore(context) == 0;
                        context = std::ptr::null();
                    } else {
                        installed = true;
                    }
                    if failed { break; }
                }
            }
            if action == "install" && !readonly {
                failed = CertAddEncodedCertificateToStore(store, X509_ASN_ENCODING,
                    cert.as_ptr(), cert.len() as u32, CERT_STORE_ADD_REPLACE_EXISTING,
                    std::ptr::null_mut()) == 0;
                if !failed { installed = true; }
            }
            CertCloseStore(store, 0);
            if failed { return Err("Windows 拒绝修改证书信任，请检查权限".into()); }
        }
        if action == "uninstall" && installed {
            return Err("当前用户信任已清理，但本地计算机仍信任此证书，请在 Windows 证书管理中以管理员权限移除".into());
        }
        Ok(installed)
    }
}
fn startup_command() -> Result<Vec<u16>> {
    let mut path = std::env::current_exe().map_err(|_| "无法取得程序路径")?;
    // Keep startup pointing at the original so upgrades refresh chrome.exe too.
    if path.file_name().is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("chrome.exe")) {
        let directory = path.parent().ok_or("无法取得程序目录")?;
        let name = std::fs::read_to_string(directory.join("data/windows-launcher.txt"))
            .map_err(|_| "无法读取原程序记录，请从原程序启动")?;
        let source = std::path::Path::new(&name);
        if source.file_name().and_then(|n| n.to_str()) != Some(name.as_str())
            || !name.to_ascii_lowercase().ends_with(".exe")
            || name.eq_ignore_ascii_case("chrome.exe") {
            return Err("原程序记录无效".into());
        }
        let original = directory.join(source);
        if !original.is_file() { return Err("找不到原程序，请恢复完整发行包".into()); }
        path = original;
    }
    Ok(format!("\"{}\"\0", path.display()).encode_utf16().collect())
}
pub fn startup(enabled: Option<bool>) -> Result<bool> {
    startup_at("Software\\Microsoft\\Windows\\CurrentVersion\\Run", &startup_command()?, enabled)
}
fn startup_at(path: &str, command: &[u16], enabled: Option<bool>) -> Result<bool> {
    unsafe {
        let path: Vec<u16> = format!("{path}\0").encode_utf16().collect();
        let name: Vec<u16> = "GBF Flash Cache\0".encode_utf16().collect();
        let mut key = std::ptr::null_mut();
        let result = RegCreateKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, std::ptr::null(), 0,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            std::ptr::null(), &mut key, std::ptr::null_mut());
        if result != 0 { return Err("无法访问开机启动设置".into()); }
        let mut bytes = vec![0u16; 32768];
        let mut length = (bytes.len() * 2) as u32;
        let mut kind = 0;
        let read = RegQueryValueExW(key, name.as_ptr(), std::ptr::null(), &mut kind,
            bytes.as_mut_ptr().cast(), &mut length);
        let active = read == 0 && kind == REG_SZ && length > 2;
        let current = active && length as usize == command.len()*2
            && bytes[..command.len()] == *command;
        // The named entry belongs to the app, not one portable directory.
        let write = match enabled {
            Some(false) if read == 0 => RegDeleteValueW(key, name.as_ptr()),
            Some(true) => RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, command.as_ptr().cast(), (command.len()*2) as u32),
            None if active && !current => RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, command.as_ptr().cast(), (command.len()*2) as u32),
            _ => 0,
        };
        RegCloseKey(key);
        if write != 0 { return Err("无法更改开机启动设置".into()); }
        Ok(enabled.unwrap_or(active))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn moved_startup_is_repaired_and_can_be_disabled() {
        let path = format!("Software\\GBF-Startup-Test-{}", std::process::id());
        let old: Vec<u16> = "\"C:\\old\\app.exe\"\0".encode_utf16().collect();
        let new: Vec<u16> = "\"C:\\new\\app.exe\"\0".encode_utf16().collect();
        assert!(startup_at(&path, &old, Some(true)).unwrap());
        assert!(startup_at(&path, &new, None).unwrap());
        unsafe {
            let key: Vec<u16> = format!("{path}\0").encode_utf16().collect();
            let name: Vec<u16> = "GBF Flash Cache\0".encode_utf16().collect();
            let mut value = vec![0u16; 1024]; let mut size = 2048;
            assert_eq!(RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ,
                std::ptr::null_mut(), value.as_mut_ptr().cast(), &mut size), 0);
            assert_eq!(&value[..size as usize/2], &new);
            assert!(!startup_at(&path, &new, Some(false)).unwrap());
            assert!(!startup_at(&path, &new, None).unwrap());
            RegDeleteKeyW(HKEY_CURRENT_USER, key.as_ptr());
        }
    }
}
