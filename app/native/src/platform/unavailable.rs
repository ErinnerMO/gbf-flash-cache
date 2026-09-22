//! Existing non-Windows behavior: no saved passwords or direct system trust changes.
use gbf_flash_cache_core::network::Result;
pub fn protect_password(_: &str) -> Result<Option<String>> {
    Ok(None)
}
pub fn unprotect_password(_: &str) -> Result<Option<String>> {
    Ok(None)
}
pub fn startup(enabled: Option<bool>) -> Result<Option<bool>> {
    if enabled.is_some() {
        return Err("当前平台不支持开机启动设置".into());
    }
    Ok(None)
}
pub fn trust(_: &[u8], action: &str) -> Result<Option<bool>> {
    if action != "status" {
        return Err("请通过系统设置管理证书信任".into());
    }
    Ok(None)
}
pub fn remove_previous_trust(_: &[u8]) -> Result<()> {
    Ok(())
}
