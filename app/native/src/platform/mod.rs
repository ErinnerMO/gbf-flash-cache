//! Compile-time platform selection. No application settings or lifecycle policies here.
#[cfg(not(target_os = "android"))]
use gbf_flash_cache_core::network::Result;
#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
pub use windows::*;
#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
pub use android::*;
#[cfg(not(windows))]
mod unavailable;
// Other hosts only run core/application checks; they are not product platforms.
#[cfg(not(any(windows, target_os = "android")))]
pub use system_roots as load_roots;
#[cfg(not(any(windows, target_os = "android")))]
pub use unavailable::*;

#[cfg(not(target_os = "android"))]
fn native_roots() -> Vec<Vec<u8>> {
    rustls_native_certs::load_native_certs()
        .certs
        .into_iter()
        .map(|cert| cert.as_ref().to_vec())
        .collect()
}
#[cfg(not(target_os = "android"))]
fn require_roots(roots: Vec<Vec<u8>>) -> Result<Vec<Vec<u8>>> {
    if roots.is_empty() {
        return Err("system trust store is empty".into());
    }
    Ok(roots)
}
#[cfg(not(target_os = "android"))]
pub fn system_roots() -> Result<Vec<Vec<u8>>> {
    require_roots(native_roots())
}
