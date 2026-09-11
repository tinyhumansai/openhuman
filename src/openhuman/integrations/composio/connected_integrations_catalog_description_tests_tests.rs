use super::resolve_toolkit_description;
use std::collections::HashMap;

#[test]
fn prefers_dynamic_catalog_description_when_present() {
    let mut catalog = HashMap::new();
    catalog.insert(
        "gmail".to_string(),
        "Live Gmail blurb from the Composio catalog".to_string(),
    );
    assert_eq!(
        resolve_toolkit_description(&catalog, "gmail"),
        "Live Gmail blurb from the Composio catalog"
    );
}

#[test]
fn falls_back_to_hardcoded_table_when_catalog_omits_toolkit() {
    // Empty catalog (older backend / no metadata) → hardcoded fallback.
    let catalog = HashMap::new();
    let got = resolve_toolkit_description(&catalog, "gmail");
    assert_eq!(
        got,
        crate::openhuman::integrations::composio::providers::toolkit_description("gmail")
    );
    assert!(!got.is_empty());
}

#[test]
fn falls_back_when_a_different_toolkit_is_catalogued() {
    // Catalog has an entry, but not for the slug we're rendering — the
    // fallback must still apply per-slug, not globally.
    let mut catalog = HashMap::new();
    catalog.insert("notion".to_string(), "Notion from catalog".to_string());
    assert_eq!(
        resolve_toolkit_description(&catalog, "gmail"),
        crate::openhuman::integrations::composio::providers::toolkit_description("gmail")
    );
}
