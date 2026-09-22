//! Platform-independent cache and network engine. Hosts supply runtime configuration.
mod directory;
pub mod memory;
pub mod refs;
pub mod scheduler;
pub mod storage;

pub const CDN: &str = "https://prd-game-a-granbluefantasy.akamaized.net";
/// Explicit GBF CDN hosts only; never match unrelated Akamai tenants.
pub const CDN_HOSTS: &[&str] = &[
    "prd-game-a-gbf.akamaized.net",
    "prd-game-a1-gbf.akamaized.net",
    "prd-game-a2-gbf.akamaized.net",
    "prd-game-a3-gbf.akamaized.net",
    "prd-game-a4-gbf.akamaized.net",
    "prd-game-a5-gbf.akamaized.net",
    "prd-game-a-granbluefantasy.akamaized.net",
    "prd-game-a1-granbluefantasy.akamaized.net",
    "prd-game-a2-granbluefantasy.akamaized.net",
    "prd-game-a3-granbluefantasy.akamaized.net",
    "prd-game-a4-granbluefantasy.akamaized.net",
    "prd-game-a5-granbluefantasy.akamaized.net",
    "prd-game-a-granbluefantasy-steam.akamaized.net",
    "prd-game-a1-granbluefantasy-steam.akamaized.net",
    "prd-game-a2-granbluefantasy-steam.akamaized.net",
    "prd-game-a3-granbluefantasy-steam.akamaized.net",
    "prd-game-a4-granbluefantasy-steam.akamaized.net",
    "prd-game-a5-granbluefantasy-steam.akamaized.net",
    "granbluefantasy.akamaized.net",
    "gbf.akamaized.net",
];
pub const BODY_LIMIT: usize = 16 * 1024 * 1024;
pub type Headers = Vec<(String, String)>;

pub fn header<'a>(headers: &'a Headers, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .rev()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}
pub fn hash(value: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(value))
}

pub mod certificates;

pub mod network;

pub mod cache;

pub mod engine;

pub mod tunnel;

pub mod gateway;

pub mod trace;

pub mod service;

pub mod udp;
