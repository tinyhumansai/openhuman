use super::connectable_toolkit_slugs;
use crate::openhuman::integrations::composio::types::ComposioToolkitCatalogEntry;

fn entry(slug: &str, enabled: bool) -> ComposioToolkitCatalogEntry {
    ComposioToolkitCatalogEntry {
        slug: slug.to_string(),
        enabled: Some(enabled),
        ..Default::default()
    }
}

#[test]
fn prefers_catalog_enabled_entries_and_drops_disabled() {
    // Disabled entries are excluded (the gate would reject them); the
    // flat `toolkits` array is ignored when a catalog is present.
    let catalog = vec![
        entry("gmail", true),
        entry("notion", false),
        entry("GitHub", true), // uppercase slug normalised
    ];
    let toolkits = vec!["ignored_when_catalog_present".to_string()];
    assert_eq!(
        connectable_toolkit_slugs(&toolkits, &catalog),
        vec!["gmail".to_string(), "github".to_string()]
    );
}

#[test]
fn falls_back_to_toolkits_when_catalog_empty() {
    // Older backends send only `toolkits`; membership must still resolve.
    let toolkits = vec!["Gmail".to_string(), "   ".to_string()];
    assert_eq!(
        connectable_toolkit_slugs(&toolkits, &[]),
        vec!["gmail".to_string()]
    );
}

#[test]
fn enabled_none_is_treated_as_not_connectable() {
    // Defensive: an entry without an explicit `enabled` is excluded,
    // matching the gate's strict `enabled === true` check.
    let catalog = vec![ComposioToolkitCatalogEntry {
        slug: "mystery".to_string(),
        enabled: None,
        ..Default::default()
    }];
    assert!(connectable_toolkit_slugs(&[], &catalog).is_empty());
}
