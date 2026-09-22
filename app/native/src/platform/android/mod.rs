//! Android roots are supplied by the Java host through AndroidCAStore.
pub use super::unavailable::*;
use gbf_flash_cache_core::network::Result;
pub fn load_roots() -> Result<Vec<Vec<u8>>> {
    Err("Android trust roots must be supplied by the platform host".into())
}

mod capture;
mod forwarder;
