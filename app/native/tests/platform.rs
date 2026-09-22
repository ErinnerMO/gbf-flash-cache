use gbf_flash_cache_app::platform;

#[test]
fn platform_capabilities_do_not_fall_back_to_plaintext_or_trust_changes() {
    let protected = platform::protect_password("test-secret").unwrap();
    #[cfg(windows)] {
        let protected = protected.unwrap();
        assert_ne!(protected, "test-secret");
        assert_eq!(platform::unprotect_password(&protected).unwrap().as_deref(), Some("test-secret"));
        assert_eq!(platform::protect_password("").unwrap().as_deref(), Some(""));
    }
    #[cfg(not(windows))] {
        assert!(protected.is_none());
        assert!(platform::unprotect_password("not-a-supported-secret").unwrap().is_none());
        assert_eq!(platform::startup(None).unwrap(), None);
        assert!(platform::startup(Some(true)).is_err());
        assert_eq!(platform::trust(b"unused", "status").unwrap(), None);
        assert!(platform::trust(b"unused", "install").is_err());
        assert!(platform::trust(b"unused", "uninstall").is_err());
    }
    assert!(!platform::load_roots().unwrap().is_empty());
}
