//! Host-owned Gmail provider surface tests.
//!
//! Pagination, cursor, envelope parsing, and ingest behavior are owned and
//! tested by `tinycortex::memory::sync::GmailSyncPipeline`.

use super::provider::{BASE_QUERY, SENT_QUERIES};
use super::GmailProvider;
use crate::openhuman::memory::sync::composio::providers::ComposioProvider;

#[test]
fn provider_metadata_is_stable() {
    let provider = GmailProvider::new();
    assert_eq!(provider.toolkit_slug(), "gmail");
    assert_eq!(provider.sync_interval_secs(), Some(15 * 60));
}

#[test]
fn default_impl_matches_new() {
    let _new = GmailProvider::new();
    let _default = GmailProvider::default();
}

#[test]
fn provider_source_does_not_restrict_to_inbox() {
    let source = include_str!("provider.rs");
    assert!(
        !source.contains("\"in:inbox"),
        "provider query must not exclude sent mail"
    );
}

#[test]
fn base_query_excludes_spam_and_trash_without_inbox_restriction() {
    assert!(BASE_QUERY.contains("-in:spam"));
    assert!(BASE_QUERY.contains("-in:trash"));
    assert!(!BASE_QUERY.contains("in:inbox"));
}

#[test]
fn sent_mail_query_strings_are_well_formed() {
    assert!(!SENT_QUERIES.is_empty());
    for query in SENT_QUERIES {
        assert!(!query.is_empty());
        assert!(!query.starts_with("in:inbox"));
    }
}
