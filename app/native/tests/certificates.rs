#![cfg(windows)]
use gbf_flash_cache_app::application::{Fields, Service};
use gbf_flash_cache_core::hash;
#[cfg(windows)]
#[tokio::test]
async fn windows_trust_is_exact_and_old_ca_is_removed() {
    let home = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let other_ca = gbf_flash_cache_core::certificates::Authority::open(other.path()).unwrap();
    let mut service = Service::open(home.path().into()).unwrap();
    service.command("ca", Fields::new()).await.unwrap();
    let old =
        gbf_flash_cache_core::certificates::Authority::stored_certificate(&home.path().join("ca"))
            .unwrap();
    // Only randomly generated test CAs are installed, then removed even on assertion failure.
    struct Cleanup(Vec<Vec<u8>>);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            for ca in &self.0 {
                let _ = gbf_flash_cache_app::platform::windows::settings::trust(ca, "uninstall");
            }
        }
    }
    let mut cleanup = Cleanup(vec![old.clone(), other_ca.certificate.clone()]);
    gbf_flash_cache_app::platform::windows::settings::trust(&other_ca.certificate, "install").unwrap();
    let installed = service.command("ca_install", Fields::new()).await.unwrap();
    assert_eq!(installed["trusted"], "true");
    let result = service
        .command(
            "ca_regenerate",
            Fields::from([("fingerprint".into(), hash(&old))]),
        )
        .await
        .unwrap();
    assert_eq!(result["trusted"], "false");
    assert!(!gbf_flash_cache_app::platform::windows::settings::trust(&old, "status").unwrap());
    assert!(gbf_flash_cache_app::platform::windows::settings::trust(&other_ca.certificate, "status").unwrap());
    cleanup.0.push(
        gbf_flash_cache_core::certificates::Authority::stored_certificate(&home.path().join("ca"))
            .unwrap(),
    );
    service.command("ca_install", Fields::new()).await.unwrap();
    assert_eq!(
        service
            .command("ca_uninstall", Fields::new())
            .await
            .unwrap()["trusted"],
        "false"
    );
    service.close().await.unwrap();
}
