use super::*;

fn entry() -> std::sync::Arc<keyring::Entry> {
    std::sync::Arc::new(keyring::Entry::new_with_credential(Box::new(
        keyring::mock::MockCredential::default(),
    )))
}

fn source(entry: &std::sync::Arc<keyring::Entry>) -> OsMasterKeySource {
    OsMasterKeySource {
        entry: Some(entry.clone()),
    }
}

#[test]
fn missing_os_entry_is_the_only_absence() {
    assert!(source(&entry()).load().unwrap().is_none());
}

#[test]
fn existing_os_master_key_keeps_its_value() {
    let entry = entry();
    entry.set_password(&"ab".repeat(32)).unwrap();
    assert_eq!(*source(&entry).load().unwrap().unwrap(), [0xab; 32]);
}

#[test]
fn malformed_os_master_key_is_rejected_without_replacement() {
    let entry = entry();
    for value in ["abcd".to_string(), "zz".repeat(32), "é".repeat(64)] {
        entry.set_password(&value).unwrap();
        assert!(source(&entry).load().is_err());
        assert_eq!(entry.get_password().unwrap(), value);
    }
}

#[test]
fn denied_and_platform_failed_entries_do_not_read_as_absent() {
    for error in [
        keyring::Error::NoStorageAccess(Box::new(std::io::Error::other("denied"))),
        keyring::Error::PlatformFailure(Box::new(std::io::Error::other("failed"))),
    ] {
        let entry = entry();
        entry
            .get_credential()
            .downcast_ref::<keyring::mock::MockCredential>()
            .unwrap()
            .set_error(error);
        assert!(source(&entry).load().is_err());
    }
}

#[test]
fn master_key_source_stores_the_existing_hex_format_and_reads_it_back() {
    let entry = entry();
    let source = source(&entry);
    source.store(&[0xcd; 32]).unwrap();
    assert_eq!(entry.get_password().unwrap(), "cd".repeat(32));
    assert_eq!(*source.load().unwrap().unwrap(), [0xcd; 32]);
}

#[test]
fn denied_master_key_write_returns_an_error() {
    let entry = entry();
    entry
        .get_credential()
        .downcast_ref::<keyring::mock::MockCredential>()
        .unwrap()
        .set_error(keyring::Error::NoStorageAccess(Box::new(
            std::io::Error::other("denied"),
        )));
    assert!(source(&entry).store(&[1; 32]).is_err());
}

#[test]
fn invalid_encoded_master_key_errors_do_not_include_secret_bytes() {
    let entry = entry();
    let bytes = b"private-master-key-payload".to_vec();
    let diagnostic_bytes = format!("{bytes:?}");
    entry
        .get_credential()
        .downcast_ref::<keyring::mock::MockCredential>()
        .unwrap()
        .set_error(keyring::Error::BadEncoding(bytes));
    let error = source(&entry).load().unwrap_err().to_string();
    assert!(!error.contains(&diagnostic_bytes));
    assert!(!error.contains("private-master-key-payload"));
}
