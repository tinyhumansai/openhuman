//! Tests for the surrounding module.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use super::*;

/// Every toolkit with a curated catalog, in display order.
const CAPABILITY_TOOLKITS: &[&str] = &[
    "gmail",
    "notion",
    "slack",
    "clickup",
    "github",
    "discord",
    "googlecalendar",
    "googledrive",
    "googledocs",
    "googlesheets",
    "outlook",
    "microsoft_teams",
    "linear",
    "jira",
    "trello",
    "asana",
    "dropbox",
    "twitter",
    "spotify",
    "telegram",
    "whatsapp",
    "shopify",
    "stripe",
    "hubspot",
    "salesforce",
    "airtable",
    "figma",
    "youtube",
    "one_drive",
    "excel",
    "todoist",
];

#[test]
fn catalog_for_toolkit_resolves_every_capability_toolkit() {
    // Every toolkit the capability surface reports on must have a catalog —
    // that is what `curated_tools: true` in the matrix claims about it.
    for toolkit in CAPABILITY_TOOLKITS {
        assert!(
            catalog_for_toolkit(toolkit).is_some(),
            "no curated catalog for advertised toolkit {toolkit}"
        );
    }
}

#[test]
fn catalog_for_toolkit_honours_slug_aliases() {
    // `toolkit_from_slug` extracts "one" from `ONE_DRIVE_*`, while the UI and
    // the backend both spell it "one_drive" / "onedrive".
    for alias in ["one", "one_drive", "onedrive", "OneDrive"] {
        assert!(
            catalog_for_toolkit(alias).is_some(),
            "OneDrive alias {alias} did not resolve"
        );
    }
    // The legacy "microsoft" alias still reaches the Teams catalog.
    assert_eq!(
        catalog_for_toolkit("microsoft").map(<[CuratedTool]>::len),
        catalog_for_toolkit("microsoft_teams").map(<[CuratedTool]>::len)
    );
    for alias in ["google_calendar", "googlecalendar", "GOOGLECALENDAR"] {
        assert!(
            catalog_for_toolkit(alias).is_some(),
            "{alias} did not resolve"
        );
    }
    assert!(
        catalog_for_toolkit("  gmail  ").is_some(),
        "slug is not trimmed"
    );
    assert!(catalog_for_toolkit("nonexistent-toolkit").is_none());
}

#[test]
fn every_native_provider_has_a_catalog() {
    for slug in NATIVE_PROVIDERS {
        assert!(
            catalog_for_toolkit(slug).is_some(),
            "native provider {slug} has no curated catalog"
        );
        assert!(has_native_provider(slug));
        assert!(
            CAPABILITY_TOOLKITS.contains(slug),
            "native provider {slug} is missing from the capability list"
        );
    }
    assert!(!has_native_provider("jira"));
    assert!(!has_native_provider("nonexistent-toolkit"));
}

#[test]
fn curated_scope_for_reads_the_catalogs_entry_not_the_heuristic() {
    // Pick a real curated read action and assert the catalog's own verdict.
    let catalog = catalog_for_toolkit("gmail").expect("gmail catalog");
    let read_action = catalog
        .iter()
        .find(|t| t.scope == ToolScope::Read)
        .expect("gmail has a curated read action");
    assert_eq!(curated_scope_for(read_action.slug), Some(ToolScope::Read));

    // An uncurated slug on a curated toolkit is `None` — deliberately not the
    // `classify_unknown` heuristic, which callers opt into explicitly.
    assert_eq!(curated_scope_for("GMAIL_NO_SUCH_ACTION_EXISTS"), None);
    // A slug with no toolkit prefix at all.
    assert_eq!(curated_scope_for("nonsense"), None);
}

#[test]
fn is_action_visible_gates_on_the_curated_scope() {
    let catalog = catalog_for_toolkit("gmail").expect("gmail catalog");
    let read_action = catalog
        .iter()
        .find(|t| t.scope == ToolScope::Read)
        .expect("gmail has a curated read action");
    let admin_action = catalog
        .iter()
        .find(|t| t.scope == ToolScope::Admin)
        .expect("gmail has a curated admin action");

    let read_only = UserScopePref {
        read: true,
        write: false,
        admin: false,
    };
    assert!(is_action_visible_with_pref(read_action.slug, &read_only));
    assert!(!is_action_visible_with_pref(admin_action.slug, &read_only));

    let all = UserScopePref {
        read: true,
        write: true,
        admin: true,
    };
    assert!(is_action_visible_with_pref(admin_action.slug, &all));

    // Uncurated action on a curated toolkit is hidden regardless of pref —
    // curation is a whitelist, so absence means "not surfaced", never
    // "fall back to the heuristic".
    assert!(!is_action_visible_with_pref("GMAIL_NO_SUCH_ACTION", &all));

    // A slug with no toolkit prefix is not ours to gate.
    assert!(is_action_visible_with_pref("nonsense", &read_only));
}

#[test]
fn toolkit_description_is_populated_for_every_capability_toolkit() {
    let generic = toolkit_description("definitely-not-a-real-toolkit");
    for toolkit in CAPABILITY_TOOLKITS {
        let d = toolkit_description(toolkit);
        assert!(!d.trim().is_empty(), "{toolkit} has an empty description");
        assert_ne!(
            d, generic,
            "{toolkit} falls through to the generic description"
        );
    }
}

#[test]
fn toolkit_description_recognizes_the_legacy_microsoft_alias() {
    // `catalog_for_toolkit` resolves both "microsoft" and "microsoft_teams" to
    // the same Teams catalog; the description must not diverge for the alias.
    assert_eq!(
        toolkit_description("microsoft"),
        toolkit_description("microsoft_teams")
    );
    assert_ne!(
        toolkit_description("microsoft"),
        toolkit_description("definitely-not-a-real-toolkit")
    );
}

#[test]
fn curated_catalogs_carry_no_duplicate_slugs() {
    // `find_curated` returns the first match, so a duplicate with a different
    // scope would make the gate's answer depend on table order.
    for toolkit in CAPABILITY_TOOLKITS {
        let catalog = catalog_for_toolkit(toolkit).expect("catalog");
        let mut seen: Vec<&str> = catalog.iter().map(|t| t.slug).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(before, seen.len(), "{toolkit} catalog has duplicate slugs");
    }
}
