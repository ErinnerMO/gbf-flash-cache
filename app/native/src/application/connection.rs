use super::Fields;
use gbf_flash_cache_core::network::Result;
use url::Url;

/// Translate saved/UI connection fields at the common start/probe boundary.
pub(super) fn resolve(args: &Fields) -> Result<Fields> {
    let upstream = proxy_address(args)?;
    let mut resolved = args.clone();
    for key in [
        "proxy",
        "protocol",
        "host",
        "proxyPort",
        "username",
        "password",
        "connection",
        "capture",
        "upstream",
    ] {
        resolved.remove(key);
    }
    if let Some(address) = upstream {
        resolved.insert("upstream".into(), address);
    }
    Ok(resolved)
}

fn proxy_address(args: &Fields) -> Result<Option<String>> {
    if args.get("proxy").is_none_or(|v| v != "true") {
        return Ok(None);
    }
    let protocol = args
        .get("protocol")
        .map_or("http", String::as_str)
        .to_ascii_lowercase();
    if !["http", "https", "socks4", "socks5"].contains(&protocol.as_str()) {
        return Err("不支持的代理协议".into());
    }
    let host = args.get("host").ok_or("请填写代理地址")?;
    let port = args
        .get("proxyPort")
        .map(|p| p.parse::<u16>().map_err(|_| "代理端口须为 1–65535"))
        .transpose()?
        .unwrap_or(0);
    if port == 0 {
        return Err("代理端口须为 1–65535".into());
    }
    let mut url = Url::parse(&format!(
        "{protocol}://{}",
        gbf_flash_cache_core::tunnel::authority(host, port)
    ))
    .map_err(|_| "代理地址无效")?;
    let user = args.get("username").map_or("", String::as_str);
    let password = if protocol == "socks4" {
        ""
    } else {
        args.get("password").map_or("", String::as_str)
    };
    validate_credentials(args)?;
    // URL setters escape delimiters but preserve percent signs; escape literal percent first.
    url.set_username(&user.replace('%', "%25"))
        .map_err(|_| "代理用户名无效")?;
    if !user.is_empty() && protocol != "socks4" {
        url.set_password(Some(&password.replace('%', "%25")))
            .map_err(|_| "代理密码无效")?;
    }
    Ok(Some(url.into()))
}

pub(super) fn validate_credentials(args: &Fields) -> Result<()> {
    if args.get("proxy").is_none_or(|v| v != "true") {
        return Ok(());
    }
    let protocol = args
        .get("protocol")
        .map_or("http", String::as_str)
        .to_ascii_lowercase();
    let user = args.get("username").map_or("", String::as_str);
    let password = if protocol == "socks4" {
        ""
    } else {
        args.get("password").map_or("", String::as_str)
    };
    gbf_flash_cache_core::tunnel::validate_proxy_credentials(&protocol, user, password)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_proxy_fields_without_leaking_ui_settings() {
        let mut args = Fields::from([
            ("proxy".into(), "true".into()),
            ("host".into(), "127.0.0.1".into()),
            ("proxyPort".into(), "7890".into()),
            ("username".into(), "user".into()),
            ("password".into(), "p@ss".into()),
            ("capture".into(), "true".into()),
        ]);
        for protocol in ["http", "https", "socks4", "socks5"] {
            args.insert("protocol".into(), protocol.into());
            let resolved = resolve(&args).unwrap();
            let url = Url::parse(&resolved["upstream"]).unwrap();
            assert_eq!(url.scheme(), protocol);
            assert_eq!(url.port(), Some(7890));
            assert!(!resolved.contains_key("password"));
            assert!(!resolved.contains_key("capture"));
            assert_eq!(url.password().is_some(), protocol != "socks4");
        }
        args.insert("proxyPort".into(), "0".into());
        assert!(resolve(&args).is_err());
        args.insert("proxy".into(), "false".into());
        args.insert("upstream".into(), "http://untrusted:8080".into());
        assert!(!resolve(&args).unwrap().contains_key("upstream"));
    }
}
