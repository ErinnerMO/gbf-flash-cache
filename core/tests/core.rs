use gbf_flash_cache_core::{certificates::Authority, storage::Entry};

#[test]
fn new_ca_persists_signs_and_rejects_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let ca = Authority::open(dir.path()).unwrap();
    let before = std::fs::read(dir.path().join("authority.json")).unwrap();
    assert_eq!(
        Authority::open(dir.path()).unwrap().certificate,
        ca.certificate
    );
    for host in [
        "127.0.0.1",
        "game.granbluefantasy.jp",
        "prd-game-a-granbluefantasy.akamaized.net",
    ] {
        let leaf = ca.leaf(host).unwrap();
        let (_, root) = x509_parser::parse_x509_certificate(&ca.certificate).unwrap();
        let (_, parsed) = x509_parser::parse_x509_certificate(&leaf.certificate).unwrap();
        parsed.verify_signature(Some(root.public_key())).unwrap();
        assert_eq!(parsed.issuer().as_raw(), root.subject().as_raw());
        assert!(parsed.validity().is_valid());
        assert!(parsed.validity().not_after <= root.validity().not_after);
    }
    let mut corrupt: serde_json::Value = serde_json::from_slice(&before).unwrap();
    corrupt["key"] = serde_json::json!([0, 1, 2]);
    assert!(Authority::from_bytes(&serde_json::to_vec(&corrupt).unwrap()).is_err());
    std::fs::write(dir.path().join("authority.json"), b"corrupt").unwrap();
    assert!(Authority::open(dir.path()).is_err());
    assert_eq!(
        std::fs::read(dir.path().join("authority.json")).unwrap(),
        b"corrupt"
    );
}

#[test]
fn cache_preserves_original_bytes_and_rejects_bad_input() {
    let entry = Entry {
        checked: 1789583019013,
        headers: vec![("Content-Type".into(), "image/png".into())],
        variant: String::new(),
        body: (0..=255).collect(),
    };
    let mut bytes = vec![];
    entry.write(&mut bytes).unwrap();
    assert_eq!(Entry::read(bytes.as_slice()).unwrap(), entry);
    for end in 0..bytes.len() {
        assert!(Entry::read(&bytes[..end]).is_err());
    }
    let mut bad = bytes.clone();
    bad[..4].copy_from_slice(&2i32.to_be_bytes());
    assert!(Entry::read(bad.as_slice()).is_err());
    let mut bad = bytes;
    bad[4..8].copy_from_slice(&i32::MAX.to_be_bytes());
    assert!(Entry::read(bad.as_slice()).is_err());
}
