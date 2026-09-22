mod credentials;
pub mod settings;
pub use super::system_roots as load_roots;
use gbf_flash_cache_core::network::Result;

pub fn protect_password(value: &str) -> Result<Option<String>> {
    if value.is_empty() {
        return Ok(Some(String::new()));
    }
    credentials::protect(value).map(Some)
}
pub fn unprotect_password(value: &str) -> Result<Option<String>> {
    credentials::unprotect(value).map(Some)
}
pub fn startup(enabled: Option<bool>) -> Result<Option<bool>> {
    settings::startup(enabled).map(Some)
}
pub fn trust(cert: &[u8], action: &str) -> Result<Option<bool>> {
    settings::trust(cert, action).map(Some)
}
pub fn remove_previous_trust(cert: &[u8]) -> Result<()> {
    settings::trust(cert, "uninstall").map(|_| ())
}
