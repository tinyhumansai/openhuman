//! Tests for the action-scope classification and the per-toolkit preference.
//!
//! The storage half — reading and writing a [`super::UserScopePref`] through
//! the key/value seam — is tested in the engine crate next to the code that
//! performs it. What is pinned here is everything two separately compiled
//! processes have to agree on: the verb precedence in
//! [`super::classify_unknown`], the multi-segment toolkit prefixes, the
//! persisted preference shape, and the default that decides what a brand-new
//! connection is allowed to do.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use super::{
    agent_ready_toolkits, classify_unknown, find_curated, toolkit_from_slug, CuratedTool,
    ToolScope, UserScopePref,
};

#[test]
fn destructive_verbs_classify_as_admin() {
    assert_eq!(classify_unknown("GMAIL_DELETE_EMAIL"), ToolScope::Admin);
    assert_eq!(classify_unknown("GMAIL_TRASH_EMAIL"), ToolScope::Admin);
    assert_eq!(classify_unknown("GMAIL_MODIFY_LABELS"), ToolScope::Admin);
    assert_eq!(classify_unknown("DRIVE_SHARE_FILE"), ToolScope::Admin);
}

#[test]
fn mutating_verbs_classify_as_write() {
    assert_eq!(classify_unknown("GMAIL_SEND_EMAIL"), ToolScope::Write);
    assert_eq!(classify_unknown("NOTION_CREATE_PAGE"), ToolScope::Write);
    assert_eq!(classify_unknown("NOTION_UPDATE_PAGE"), ToolScope::Write);
}

#[test]
fn anything_else_classifies_as_read() {
    assert_eq!(classify_unknown("GMAIL_FETCH_EMAILS"), ToolScope::Read);
    assert_eq!(classify_unknown("NOTION_SEARCH"), ToolScope::Read);
    assert_eq!(classify_unknown("GMAIL_GET_PROFILE"), ToolScope::Read);
}

#[test]
fn admin_verbs_are_checked_before_write_verbs() {
    // `DELETE_DRAFT` contains `DRAFT`, a write verb. If the two lists were
    // checked in the other order this would gate as a write and a destructive
    // action would run under a write-only preference.
    assert_eq!(classify_unknown("GMAIL_DELETE_DRAFT"), ToolScope::Admin);
}

#[test]
fn classification_ignores_slug_casing() {
    assert_eq!(classify_unknown("gmail_delete_email"), ToolScope::Admin);
    assert_eq!(classify_unknown("gmail_send_email"), ToolScope::Write);
}

#[test]
fn a_toolkit_slug_is_the_lowercased_first_segment() {
    assert_eq!(
        toolkit_from_slug("GMAIL_SEND_EMAIL").as_deref(),
        Some("gmail")
    );
    assert_eq!(
        toolkit_from_slug("NOTION_FETCH_DATA").as_deref(),
        Some("notion")
    );
    assert_eq!(
        toolkit_from_slug("noUnderscore").as_deref(),
        Some("nounderscore")
    );
}

#[test]
fn an_empty_slug_names_no_toolkit() {
    assert_eq!(toolkit_from_slug(""), None);
    assert_eq!(toolkit_from_slug("   "), None);
}

#[test]
fn multi_segment_toolkits_keep_their_whole_prefix() {
    // Without these three, `ZOHO_MAIL_*` resolves to `zoho`, matches no
    // connected toolkit, and every action for it is silently dropped.
    assert_eq!(
        toolkit_from_slug("ZOHO_MAIL_SEND_EMAIL").as_deref(),
        Some("zoho_mail")
    );
    assert_eq!(
        toolkit_from_slug("ONE_DRIVE_GET_FILE").as_deref(),
        Some("one_drive")
    );
    assert_eq!(
        toolkit_from_slug("MICROSOFT_TEAMS_SEND_MESSAGE").as_deref(),
        Some("microsoft_teams")
    );
}

#[test]
fn a_curated_lookup_ignores_casing_and_reports_a_miss() {
    let catalog = &[CuratedTool {
        slug: "GMAIL_SEND_EMAIL",
        scope: ToolScope::Write,
    }];
    assert!(find_curated(catalog, "gmail_send_email").is_some());
    assert!(find_curated(catalog, "GMAIL_SEND_EMAIL").is_some());
    assert!(find_curated(catalog, "GMAIL_DELETE_EMAIL").is_none());
}

#[test]
fn every_tool_scope_tag_matches_its_serde_form() {
    for scope in [ToolScope::Read, ToolScope::Write, ToolScope::Admin] {
        let json = serde_json::to_string(&scope).expect("serialize");
        assert_eq!(json, format!("\"{}\"", scope.as_str()));
    }
}

#[test]
fn a_new_connection_may_read_and_write_but_not_administer() {
    let pref = UserScopePref::default();
    assert!(pref.read);
    assert!(pref.write);
    assert!(!pref.admin);
}

#[test]
fn allows_answers_per_scope() {
    let pref = UserScopePref {
        read: true,
        write: false,
        admin: false,
    };
    assert!(pref.allows(ToolScope::Read));
    assert!(!pref.allows(ToolScope::Write));
    assert!(!pref.allows(ToolScope::Admin));
}

#[test]
fn a_preference_round_trips() {
    let pref = UserScopePref {
        read: true,
        write: true,
        admin: true,
    };
    let value = serde_json::to_value(pref).expect("serialize");
    let back: UserScopePref = serde_json::from_value(value).expect("deserialize");
    assert_eq!(pref, back);
}

#[test]
fn a_row_missing_read_and_write_decodes_as_permitted_not_denied() {
    // A stored row written before a field existed must not read back as a
    // denial: `#[serde(default)]` on a `bool` would silently revoke access the
    // user never revoked.
    let stored = serde_json::json!({ "admin": true });
    let pref: UserScopePref = serde_json::from_value(stored).expect("deserialize");
    assert!(pref.read);
    assert!(pref.write);
    assert!(pref.admin);
}

#[test]
fn the_agent_ready_list_is_sorted_and_free_of_duplicates() {
    // The RPC response has to be stable across builds, and the badge logic is a
    // membership test — a duplicate would be invisible there and confusing in
    // the panel.
    let slugs = agent_ready_toolkits();
    let mut sorted = slugs.clone();
    sorted.sort_unstable();
    assert_eq!(slugs, sorted, "the list must come back sorted");

    let mut deduped = sorted.clone();
    deduped.dedup();
    assert_eq!(deduped.len(), slugs.len(), "the list has a duplicate slug");
}

#[test]
fn the_agent_ready_list_names_the_native_providers() {
    let slugs = agent_ready_toolkits();
    for native in ["gmail", "notion", "github", "linear"] {
        assert!(slugs.contains(&native), "{native} is missing from the list");
    }
}
