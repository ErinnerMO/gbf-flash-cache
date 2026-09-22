use crate::{hash, header, Headers, BODY_LIMIT};
use regex::Regex;
use std::{
    collections::BTreeSet,
    io::{self, Read, Write},
    sync::LazyLock,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub checked: i64,
    pub headers: Headers,
    pub variant: String,
    pub body: Vec<u8>,
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid cache entry")
}
fn int(input: &mut impl Read) -> io::Result<i32> {
    let mut b = [0; 4];
    input.read_exact(&mut b)?;
    Ok(i32::from_be_bytes(b))
}
impl Entry {
    pub fn read(mut input: impl Read) -> io::Result<Self> {
        let mut magic = [0; 4];
        input.read_exact(&mut magic)?;
        if &magic != b"GFC1" {
            return Err(invalid());
        }
        let length = int(&mut input)?;
        if !(0..=65536).contains(&length) {
            return Err(invalid());
        }
        let mut metadata = vec![0; length as usize];
        input.read_exact(&mut metadata)?;
        let (checked, headers, variant): (i64, Headers, String) =
            serde_json::from_slice(&metadata).map_err(|_| invalid())?;
        if headers.len() > 128
            || headers.iter().any(|(name, value)| {
                name.is_empty()
                    || !name.bytes().all(|c| c > 32 && c < 127)
                    || !value.bytes().all(|c| c == 9 || (32..127).contains(&c))
            })
        {
            return Err(invalid());
        }
        let size = int(&mut input)?;
        if size < 0 || size as usize > BODY_LIMIT {
            return Err(invalid());
        }
        let mut body = vec![0; size as usize];
        input.read_exact(&mut body)?;
        Ok(Self {
            checked,
            headers,
            variant,
            body,
        })
    }
    pub fn write(&self, mut output: impl Write) -> io::Result<()> {
        if self.headers.len() > 128 || self.body.len() > BODY_LIMIT {
            return Err(invalid());
        }
        let metadata = serde_json::to_vec(&(self.checked, &self.headers, &self.variant))?;
        if metadata.len() > 65536 {
            return Err(invalid());
        }
        output.write_all(b"GFC1")?;
        output.write_all(&(metadata.len() as i32).to_be_bytes())?;
        output.write_all(&metadata)?;
        output.write_all(&(self.body.len() as i32).to_be_bytes())?;
        output.write_all(&self.body)
    }
    pub fn matches(&self, request: &Headers) -> bool {
        self.variant == variant(&self.headers, request)
    }
    pub fn not_modified(&self, request: &Headers) -> bool {
        static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?:W/)?"[^"]*"|\*"#).unwrap());
        if let Some(condition) = header(request, "If-None-Match") {
            return
            TAG.find_iter(condition).any(|m| {
                m.as_str() == "*"
                    || header(&self.headers, "ETag").is_some_and(|tag| {
                        m.as_str().strip_prefix("W/").unwrap_or(m.as_str())
                            == tag.strip_prefix("W/").unwrap_or(tag)
                    })
            });
        }
        match (header(request, "If-Modified-Since"), header(&self.headers, "Last-Modified")) {
            (Some(condition), Some(modified)) => match (httpdate::parse_http_date(condition), httpdate::parse_http_date(modified)) {
                (Ok(condition), Ok(modified)) => modified <= condition,
                _ => false,
            },
            _ => false,
        }
    }
}
pub fn variant(response: &Headers, request: &Headers) -> String {
    let names: BTreeSet<_> = response
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case("Vary"))
        .flat_map(|(_, v)| v.split(','))
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let mut identity = String::new();
    for name in names {
        let values: Vec<_> = request
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(&name))
            .map(|(_, v)| v.as_str())
            .collect();
        identity.push_str(&name);
        identity.push(':');
        if values.is_empty() {
            identity.push_str("absent");
        } else {
            identity.push_str("present:");
            identity.push_str(&values.join("\n"));
        }
        identity.push('\n');
    }
    hash(identity.as_bytes())
}

#[cfg(test)]
mod condition_tests {
    use super::*;
    #[test]
    fn dates_and_etag_precedence() {
        let entry = Entry { checked: 0, variant: String::new(), body: vec![], headers: vec![
            ("Last-Modified".into(), "Mon, 21 Sep 2026 00:00:00 GMT".into()),
            ("ETag".into(), "\"current\"".into()),
        ]};
        for (date, expected) in [("Mon, 21 Sep 2026 00:00:00 GMT", true), ("Tue, 22 Sep 2026 00:00:00 GMT", true), ("Sun, 20 Sep 2026 00:00:00 GMT", false), ("invalid", false)] {
            let mut headers = vec![("If-Modified-Since".into(), date.into())];
            assert_eq!(entry.not_modified(&headers), expected);
            headers.push(("If-None-Match".into(), "\"other\"".into()));
            assert!(!entry.not_modified(&headers));
            headers[1].1 = "W/\"current\"".into();
            assert!(entry.not_modified(&headers));
        }
    }
}
