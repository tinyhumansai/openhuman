use super::*;

#[test]
fn defaults_to_openhuman() {
    let _guard = product_identity_test_lock();
    reset_product_identity_for_test();
    assert_eq!(product_identity().as_str(), "openhuman");
    assert_eq!(
        ProductIdentity::default().as_str(),
        DEFAULT_PRODUCT_IDENTITY
    );
}

#[test]
fn override_is_respected_and_restorable() {
    let _guard = product_identity_test_lock();
    reset_product_identity_for_test();

    set_product_identity(ProductIdentity::new("opencompany").unwrap());
    assert_eq!(product_identity().as_str(), "opencompany");

    reset_product_identity_for_test();
    assert_eq!(product_identity().as_str(), "openhuman");
}

#[test]
fn new_keeps_allowed_characters_and_trims() {
    assert_eq!(
        ProductIdentity::new("  opencompany  ").unwrap().as_str(),
        "opencompany"
    );
    assert_eq!(
        ProductIdentity::new("open_company-2.0").unwrap().as_str(),
        "open_company-2.0"
    );
}

#[test]
fn new_drops_header_unsafe_characters() {
    // A newline would otherwise let a caller inject a second header.
    assert_eq!(
        ProductIdentity::new("opencompany\r\nx-admin: 1")
            .unwrap()
            .as_str(),
        "opencompanyx-admin1"
    );
    assert_eq!(
        ProductIdentity::new("open company").unwrap().as_str(),
        "opencompany"
    );
}

#[test]
fn new_lowercases_so_the_backend_enum_still_matches() {
    assert_eq!(
        ProductIdentity::new("OpenCompany").unwrap().as_str(),
        "opencompany"
    );
}

#[test]
fn new_rejects_values_with_nothing_usable_left() {
    assert!(ProductIdentity::new("").is_none());
    assert!(ProductIdentity::new("   ").is_none());
    assert!(ProductIdentity::new("!@#$%").is_none());
}

#[test]
fn new_truncates_overlong_values() {
    let identity = ProductIdentity::new(&"a".repeat(PRODUCT_IDENTITY_MAX_LEN * 2)).unwrap();
    assert_eq!(identity.as_str().len(), PRODUCT_IDENTITY_MAX_LEN);
}

#[test]
fn header_carries_the_current_identity() {
    let _guard = product_identity_test_lock();
    reset_product_identity_for_test();

    let headers = product_identity_headers();
    assert_eq!(
        headers.get(PRODUCT_IDENTITY_HEADER).unwrap(),
        DEFAULT_PRODUCT_IDENTITY
    );

    set_product_identity(ProductIdentity::new("opencompany").unwrap());
    let headers = product_identity_headers();
    assert_eq!(headers.get(PRODUCT_IDENTITY_HEADER).unwrap(), "opencompany");

    reset_product_identity_for_test();
}
