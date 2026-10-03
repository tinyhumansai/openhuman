//! Tests for the surrounding module.

use super::*;

#[test]
fn one_drive_catalog_is_non_empty_and_unique() {
    assert!(!ONE_DRIVE_CURATED.is_empty());
    let mut slugs: Vec<&'static str> = ONE_DRIVE_CURATED.iter().map(|t| t.slug).collect();
    slugs.sort_unstable();
    slugs.dedup();
    assert_eq!(slugs.len(), ONE_DRIVE_CURATED.len());
    for tool in ONE_DRIVE_CURATED {
        assert!(tool.slug.starts_with("ONE_DRIVE_"));
    }
}

#[test]
fn excel_catalog_is_non_empty_and_unique() {
    assert!(!EXCEL_CURATED.is_empty());
    let mut slugs: Vec<&'static str> = EXCEL_CURATED.iter().map(|t| t.slug).collect();
    slugs.sort_unstable();
    slugs.dedup();
    assert_eq!(slugs.len(), EXCEL_CURATED.len());
    for tool in EXCEL_CURATED {
        assert!(tool.slug.starts_with("EXCEL_"));
    }
}

#[test]
fn one_drive_catalog_covers_all_three_scopes() {
    assert!(ONE_DRIVE_CURATED.iter().any(|t| t.scope == ToolScope::Read));
    assert!(ONE_DRIVE_CURATED
        .iter()
        .any(|t| t.scope == ToolScope::Write));
    assert!(ONE_DRIVE_CURATED
        .iter()
        .any(|t| t.scope == ToolScope::Admin));
}

#[test]
fn excel_catalog_covers_all_three_scopes() {
    assert!(EXCEL_CURATED.iter().any(|t| t.scope == ToolScope::Read));
    assert!(EXCEL_CURATED.iter().any(|t| t.scope == ToolScope::Write));
    assert!(EXCEL_CURATED.iter().any(|t| t.scope == ToolScope::Admin));
}
