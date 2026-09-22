//! Android settings adapter for shared UDP transport.
pub use gbf_flash_cache_core::udp::Route;
use std::{collections::BTreeMap, io};
pub fn from_fields(fields: &BTreeMap<String, String>) -> io::Result<Route> {
    if fields.get("proxy").map(String::as_str) != Some("true") {
        return Ok(Route::Direct);
    }
    match fields
        .get("protocol")
        .map_or("HTTP", String::as_str)
        .to_ascii_uppercase()
        .as_str()
    {
        "HTTP" | "HTTPS" | "SOCKS4" => return Ok(Route::Direct),
        "SOCKS5" => {}
        _ => return Err(io::Error::other("不支持的代理协议")),
    }
    let host = fields
        .get("host")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| io::Error::other("请填写上游地址"))?
        .trim_matches(['[', ']'])
        .to_owned();
    let port = fields
        .get("proxyPort")
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|p| *p != 0)
        .ok_or_else(|| io::Error::other("无效的上游端口"))?;
    let user = fields.get("username").cloned().unwrap_or_default();
    let password = fields.get("password").cloned().unwrap_or_default();
    gbf_flash_cache_core::tunnel::validate_proxy_credentials("socks5", &user, &password)
        .map_err(|_| io::Error::other("无效的 SOCKS5 凭据"))?;
    Ok(Route::Socks {
        host,
        port,
        user,
        password,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_routes() {
        let mut fields = BTreeMap::from([
            ("proxy".into(), "true".into()),
            ("protocol".into(), "HTTP".into()),
            ("host".into(), "127.0.0.1".into()),
            ("proxyPort".into(), "7890".into()),
        ]);
        for protocol in ["HTTP", "HTTPS", "SOCKS4"] {
            fields.insert("protocol".into(), protocol.into());
            assert!(matches!(from_fields(&fields).unwrap(), Route::Direct));
        }
        fields.insert("protocol".into(), "SOCKS5".into());
        assert!(from_fields(&fields).is_ok());
        fields.insert("username".into(), "name".into());
        assert!(from_fields(&fields).is_err());
        fields.insert("proxy".into(), "false".into());
        assert!(matches!(from_fields(&fields).unwrap(), Route::Direct));
    }
}
