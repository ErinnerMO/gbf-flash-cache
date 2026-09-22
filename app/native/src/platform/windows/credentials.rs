use gbf_flash_cache_core::network::Result;
use base64::Engine;
use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::*};

// DPAPI binds saved secrets to the current Windows user, without a bundled key.
fn crypt(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB { cbData: bytes.len().try_into().map_err(|_| "密码过长")?, pbData: bytes.as_ptr() as *mut u8 };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        let ok = if encrypt {
            CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        } else {
            CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output)
        };
        if ok == 0 { return Err("无法使用当前 Windows 账号保护或读取代理密码".into()); }
        let result = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        std::ptr::write_bytes(output.pbData, 0, output.cbData as usize);
        LocalFree(output.pbData.cast());
        Ok(result)
    }
}
pub fn protect(value: &str) -> Result<String> {
    Ok(base64::engine::general_purpose::STANDARD.encode(crypt(value.as_bytes(), true)?))
}
pub fn unprotect(value: &str) -> Result<String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(value).map_err(|_| "代理密码数据损坏")?;
    String::from_utf8(crypt(&bytes, false)?).map_err(|_| "代理密码数据损坏".into())
}
