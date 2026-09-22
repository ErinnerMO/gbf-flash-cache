use flate2::read::{GzDecoder, ZlibDecoder};
use indexmap::IndexMap;
use regex::Regex;
use std::{collections::HashMap, io::Read, sync::LazyLock};
use url::Url;

pub const TEXT_LIMIT: usize = 2 * 1024 * 1024;
pub type References = IndexMap<String, bool>; // true means deferred/weak dependency
static ASSET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^/assets(?:_en)?/.+\.(?:png|jpe?g|gif|webp|js|css|json|mp3|ogg|m4a|woff2?|wasm)$")
        .unwrap()
});
static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"//[^\n]*|/\*[\s\S]*?\*/|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|`(?:\\.|[^`\\])*`|[\w$]+|[^\s]"#).unwrap()
});
static MARKUP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?:src|href|poster)\s*=\s*['"]([^'"]+)['"]|url\(\s*['"]?([^\s'")]+)|@import\s+['"]([^'"]+)['"]"#).unwrap()
});
static SRCSET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)srcset\s*=\s*['"]([^'"]+)['"]"#).unwrap());
static IMAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"Game\.imgUri\s*\+\s*['"](/sp/[A-Za-z0-9_/-]+\.(?:png|jpg|jpeg|webp))['"]"#)
        .unwrap()
});
static ROOT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(https://[^/]+/assets(?:_en)?/[0-9]+/js/)").unwrap());
static MODULE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_@./-]+$").unwrap());

pub struct ResourceRefs {
    origin: Url,
    aliases: HashMap<String, HashMap<String, String>>,
    pub reason: &'static str,
    pub observed_origins: Vec<String>,
}
impl ResourceRefs {
    pub fn new(origin: &str) -> Result<Self, url::ParseError> {
        Ok(Self {
            origin: Url::parse(origin)?,
            aliases: HashMap::new(),
            reason: "binary",
            observed_origins: Vec::new(),
        })
    }
    pub fn asset(&self, value: &str) -> bool {
        value.parse::<http::Uri>().is_ok_and(|u| {
            u.scheme_str() == Some(self.origin.scheme())
                && u.host().is_some_and(|host| {
                    self.origin.host_str().is_some_and(|origin| host.eq_ignore_ascii_case(origin))
                        || (self.origin.as_str().trim_end_matches('/') == crate::CDN
                            && crate::CDN_HOSTS.iter().any(|cdn| host.eq_ignore_ascii_case(cdn)))
                })
                && u.port_u16().or_else(|| match u.scheme_str() {
                    Some("https") => Some(443), Some("http") => Some(80), _ => None,
                }) == self.origin.port_or_known_default()
                && u.authority().is_some_and(|a| !a.as_str().contains('@'))
                && !value.contains('#')
                && ASSET.is_match(u.path())
        })
    }

    fn add(&self, refs: &mut References, base: &Url, value: &str, weak: bool) {
        // Unexpanded templates are not resource paths; never download placeholders.
        let decoded = percent_encoding::percent_decode_str(value).decode_utf8_lossy();
        if ["<%", "%>", "${", "{{", "}}"].iter().any(|marker| decoded.contains(marker)) { return; }
        if let Ok(mut target) = base.join(value) {
            target.set_fragment(None);
            if self.asset(target.as_str()) {
                refs.entry(target.into())
                    .and_modify(|v| *v &= weak)
                    .or_insert(weak);
            }
        }
    }
    pub fn parse(&mut self, source: &str, mime: &str, encoding: &str, body: &[u8]) -> References {
        let mut refs = References::new();
        self.reason = "binary";
        self.observed_origins.clear();
        let Ok(base) = Url::parse(source) else {
            self.reason = "invalid_url";
            return refs;
        };
        let path = base.path();
        let mime = mime.to_ascii_lowercase();
        let script = path.ends_with(".js") || mime.contains("javascript");
        let json = path.ends_with(".json") || mime.contains("json");
        let html = mime.contains("text/html");
        let css = path.ends_with(".css") || mime.contains("text/css");
        if !(script || json || html || css) {
            return refs;
        }
        self.reason = "parsed";
        let decoded = match decode(encoding, body) {
            Ok(body) => body,
            Err(reason) => { self.reason = reason; return refs; }
        };
        let text = String::from_utf8_lossy(&decoded);
        let mut tokens = Vec::new();
        let mut fragments = String::new();
        for token in TOKEN.find_iter(&text).map(|t| t.as_str()) {
            if token.starts_with("//") || token.starts_with("/*") {
                continue;
            }
            tokens.push(token);
            if let Some(value) = literal(token) {
                // Only origins, never response text, credentials, paths or query values.
                if self.observed_origins.len() < 16 && ["http://", "https://", "//"].iter().any(|prefix| value.starts_with(prefix)) {
                    if let Ok(target) = base.join(&value) {
                        let origin = target.origin().ascii_serialization();
                        if !self.observed_origins.contains(&origin) { self.observed_origins.push(origin); }
                    }
                }
                if value.starts_with("https://")
                    || value.starts_with("//")
                    || value.starts_with("/assets")
                {
                    self.add(&mut refs, &base, &value, false);
                }
                if json && fragments.len() + value.len() < TEXT_LIMIT {
                    fragments.push('\n');
                    fragments.push_str(&value);
                }
            }
        }
        let markup = format!("{text}{fragments}");
        if html || json || css {
            for m in MARKUP.captures_iter(&markup) {
                self.add(
                    &mut refs,
                    &base,
                    m.get(1)
                        .or_else(|| m.get(2))
                        .or_else(|| m.get(3))
                        .unwrap()
                        .as_str(),
                    false,
                );
            }
            for m in SRCSET.captures_iter(&markup) {
                for part in m[1].split(',') {
                    if let Some(value) = part.split_whitespace().next() {
                        self.add(&mut refs, &base, value, false);
                    }
                }
            }
        }
        for m in IMAGE.captures_iter(&markup) {
            let prefix = if path.starts_with("/assets_en/") {
                "/assets_en/img"
            } else {
                "/assets/img"
            };
            self.add(
                &mut refs,
                &self.origin,
                &format!("{prefix}{}", &m[1]),
                false,
            );
        }
        let Some(root) = ROOT
            .captures(source)
            .filter(|_| script)
            .map(|m| m[1].to_owned())
        else {
            return refs;
        };
        let names = self.aliases.entry(root.clone()).or_default();
        if path.ends_with("/require-config.js") {
            for i in 0..tokens.len().saturating_sub(3) {
                if at(&tokens, i, "paths") && at(&tokens, i + 1, ":") && at(&tokens, i + 2, "{") {
                    let mut j = i + 3;
                    while j + 2 < tokens.len() && at(&tokens, j + 1, ":") {
                        let Some(value) = literal(tokens[j + 2]) else {
                            break;
                        };
                        names.insert(
                            literal(tokens[j]).unwrap_or_else(|| tokens[j].into()),
                            value,
                        );
                        j += 3;
                        if !at(&tokens, j, ",") {
                            break;
                        }
                        j += 1;
                    }
                }
            }
        }
        // Clone the small alias map so inserting references never borrows mutable parser state.
        let names = names.clone();
        let root_url = Url::parse(&root).unwrap();
        for (i, token) in tokens.iter().enumerate() {
            if matches!(*token, "define" | "require" | "requireAMD" | "requireESM")
                && at(&tokens, i + 1, "(")
                && (i == 0 || !at(&tokens, i - 1, "."))
            {
                let mut j = i + 2;
                if *token == "define" && tokens.get(j).and_then(|t| literal(t)).is_some() {
                    j += 2;
                }
                if at(&tokens, j, "[") {
                    j += 1;
                    let mut deps = Vec::new();
                    while let Some(name) = tokens.get(j).and_then(|t| literal(t)) {
                        deps.push(name);
                        j += 1;
                        if !at(&tokens, j, ",") {
                            break;
                        }
                        j += 1;
                    }
                    if at(&tokens, j, "]") {
                        for mut name in deps {
                            if !MODULE.is_match(&name)
                                || name.starts_with(['.', '/'])
                                || name.ends_with(".js")
                                || matches!(name.as_str(), "require" | "exports" | "module")
                            {
                                continue;
                            }
                            if let Some(prefix) = names
                                .keys()
                                .filter(|a| name == **a || name.starts_with(&format!("{a}/")))
                                .max_by_key(|a| a.len())
                            {
                                name = format!("{}{}", names[prefix], &name[prefix.len()..]);
                            } else if !name.contains('/') {
                                continue;
                            }
                            self.add(
                                &mut refs,
                                &root_url,
                                &format!("{name}.js"),
                                *token != "define",
                            );
                        }
                    }
                }
            }
            let mut value = if matches!(*token, "from" | "import") {
                tokens.get(i + 1).and_then(|t| literal(t))
            } else {
                None
            };
            let mut weak = false;
            if *token == "import" && at(&tokens, i + 1, "(") && at(&tokens, i + 3, ")") {
                value = tokens.get(i + 2).and_then(|t| literal(t));
                weak = true;
            }
            if let Some(value) = value.filter(|v| v.starts_with("./") || v.starts_with("../")) {
                self.add(&mut refs, &base, &value, weak);
            }
        }
        refs
    }
}
fn decode(encoding: &str, body: &[u8]) -> Result<Vec<u8>, &'static str> {
    // Bound both decoded text and coding depth; decoding is only for observation.
    if encoding.split(',').count() > 8 { return Err("unsupported_encoding"); }
    let mut decoded = std::borrow::Cow::Borrowed(body);
    for coding in encoding.split(',').rev() {
        let input = decoded.as_ref();
        let reader: Box<dyn Read + '_> = match coding.trim().to_ascii_lowercase().as_str() {
            "" | "identity" => Box::new(input),
            "gzip" => Box::new(GzDecoder::new(input)),
            "deflate" => Box::new(ZlibDecoder::new(input)),
            "br" => Box::new(brotli::Decompressor::new(input, 4096)),
            "zstd" => {
                let mut decoder = zstd::stream::read::Decoder::new(input).map_err(|_| "decode_error")?;
                // HTTP zstd uses a window of at most 8 MiB (RFC 9659).
                decoder.window_log_max(23).map_err(|_| "decode_error")?;
                Box::new(decoder)
            }
            _ => return Err("unsupported_encoding"),
        };
        let mut output = Vec::new();
        reader.take((TEXT_LIMIT + 1) as u64).read_to_end(&mut output).map_err(|_| "decode_error")?;
        if output.len() > TEXT_LIMIT { return Err("text_limit"); }
        decoded = std::borrow::Cow::Owned(output);
    }
    Ok(decoded.into_owned())
}
fn at(tokens: &[&str], i: usize, token: &str) -> bool {
    tokens.get(i) == Some(&token)
}
fn literal(token: &str) -> Option<String> {
    if token.len() < 2 || !token.starts_with(['\'', '"']) {
        return None;
    }
    let mut units = Vec::new();
    let mut chars = token[1..token.len() - 1].chars();
    while let Some(mut c) = chars.next() {
        if c == '\\' {
            c = chars.next()?;
            if c == 'u' {
                let digits: String = chars.by_ref().take(4).collect();
                if digits.len() != 4 {
                    return None;
                }
                units.push(u16::from_str_radix(&digits, 16).ok()?);
                continue;
            }
            c = match c {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                other => other,
            };
        }
        units.extend(c.encode_utf16(&mut [0; 2]).iter().copied());
    }
    Some(String::from_utf16_lossy(&units))
}

#[cfg(test)]
mod template_tests {
    use super::*;
    use std::io::Write;
    fn encoded(coding: &str, input: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        match coding {
            "gzip" => { let mut e = flate2::write::GzEncoder::new(&mut output, flate2::Compression::fast()); e.write_all(input).unwrap(); e.finish().unwrap(); }
            "deflate" => { let mut e = flate2::write::ZlibEncoder::new(&mut output, flate2::Compression::fast()); e.write_all(input).unwrap(); e.finish().unwrap(); }
            "br" => { let mut e = brotli::CompressorWriter::new(&mut output, 4096, 1, 22); e.write_all(input).unwrap(); }
            "zstd" => output = zstd::stream::encode_all(input, 1).unwrap(),
            _ => unreachable!(),
        }
        output
    }
    #[test]
    fn compressed_observation_preserves_input_and_bounds_decoding() {
        let body = br#"{"image":"/assets/child.png"}"#;
        let source = format!("{}/assets/source.json", crate::CDN);
        let mut parser = ResourceRefs::new(crate::CDN).unwrap();
        for coding in ["gzip", "deflate", "br", "zstd"] {
            let compressed = encoded(coding, body);
            let original = compressed.clone();
            let refs = parser.parse(&source, "application/json", coding, &compressed);
            assert!(refs.contains_key(&format!("{}/assets/child.png", crate::CDN)), "{coding}");
            assert_eq!(compressed, original);
            let bomb = encoded(coding, &vec![b' '; TEXT_LIMIT + 1]);
            assert!(parser.parse(&source, "application/json", coding, &bomb).is_empty());
            assert_eq!(parser.reason, "text_limit", "{coding}");
            assert!(parser.parse(&source, "application/json", coding, b"invalid compressed data").is_empty());
            assert_eq!(parser.reason, "decode_error", "{coding}");
        }
        let layered = encoded("br", &encoded("gzip", body));
        assert_eq!(decode("gzip, Br", &layered).unwrap(), body);
        assert_eq!(decode("unknown", body), Err("unsupported_encoding"));
    }
    #[test]
    fn diagnostic_origins_exclude_credentials_paths_and_reset() {
        let mut parser = ResourceRefs::new(crate::CDN).unwrap();
        let source = "https://game.granbluefantasy.jp/";
        let body = br#"["https://user:secret@other.example/private?token=secret"]"#;
        assert!(parser.parse(source, "application/json", "", body).is_empty());
        assert_eq!(parser.observed_origins, ["https://other.example"]);
        let many = (0..32).map(|i| format!("https://h{i}.example/secret")).collect::<Vec<_>>();
        parser.parse(source, "application/json", "", &serde_json::to_vec(&many).unwrap());
        assert_eq!(parser.observed_origins.len(), 16);
        parser.parse(source, "image/png", "", b"");
        assert!(parser.observed_origins.is_empty());
    }
    #[test]
    fn unexpanded_templates_are_not_download_candidates() {
        let mut parser = ResourceRefs::new(crate::CDN).unwrap();
        let source = format!("{}/assets/templates.json", crate::CDN);
        let body = br#"["/assets/img/<%= item.id %>.png", "/assets/img/%3C%25%3D%20id%20%25%3E.jpg", "/assets/img/${id}.png", "/assets/img/{{id}}.png", "/assets/img/ok.png"]"#;
        let refs = parser.parse(&source, "application/json", "", body);
        assert_eq!(refs.keys().collect::<Vec<_>>(), vec![&format!("{}/assets/img/ok.png", crate::CDN)]);
    }
}
