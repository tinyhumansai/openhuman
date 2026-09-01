//! Tests for the surrounding module.

use super::*;

/// The two GitHub coordinate helpers are re-exported from `tinymemory-sources`
/// and they deliberately differ: `tree_scope` slugifies to
/// `github-tinyhumansai-openhuman` while `archive_source_id` slugifies to
/// `github-com-tinyhumansai-openhuman`. Swapping the two still compiles and
/// still type-checks — it just makes reconcile scan an empty directory at
/// runtime. Pin both spellings, as the engine's own test did.
#[test]
fn derive_scopes_keeps_github_tree_and_archive_ids_distinct() {
    let source: MemorySourceEntry = serde_json::from_value(serde_json::json!({
        "id": "gh-scope",
        "kind": "github_repo",
        "label": "Repo",
        "url": "https://github.com/tinyhumansai/openhuman",
    }))
    .expect("github source entry");

    let scopes = derive_scopes(&source, &Config::default());

    assert_eq!(scopes.len(), 1);
    assert_eq!(scopes[0].tree_scope, "github:tinyhumansai/openhuman");
    assert_eq!(
        scopes[0].archive_source_id,
        "github.com/tinyhumansai/openhuman"
    );
}

/// A GitHub entry with no URL has no coordinates to derive, and a kind with no
/// raw archive has nothing to reconcile. Both answer with no scopes rather than
/// with a guessed one.
#[test]
fn derive_scopes_is_empty_without_coordinates() {
    let mut source: MemorySourceEntry = serde_json::from_value(serde_json::json!({
        "id": "gh-no-url",
        "kind": "github_repo",
        "label": "Repo",
    }))
    .expect("github source entry");
    assert!(derive_scopes(&source, &Config::default()).is_empty());

    source.url = Some("not-a-url".into());
    assert!(derive_scopes(&source, &Config::default()).is_empty());

    let folder: MemorySourceEntry = serde_json::from_value(serde_json::json!({
        "id": "folder",
        "kind": "folder",
        "label": "Folder",
        "path": "/tmp",
    }))
    .expect("folder source entry");
    assert!(derive_scopes(&folder, &Config::default()).is_empty());
}

/// A Gmail connector's scope is read off the archive the sync wrote, so an
/// absent `raw/` directory yields nothing rather than failing, and a toolkit
/// with no raw archive yet yields nothing either.
#[test]
fn derive_scopes_reads_the_gmail_scope_from_the_raw_archive() {
    let tmp = tempfile::TempDir::new().expect("temp workspace");
    let config = Config {
        workspace_dir: tmp.path().to_path_buf(),
        ..Config::default()
    };
    let source: MemorySourceEntry = serde_json::from_value(serde_json::json!({
        "id": "gmail-src",
        "kind": "composio",
        "label": "Gmail",
        "toolkit": "gmail",
        "connection_id": "conn-1",
    }))
    .expect("composio source entry");

    // Nothing on disk yet.
    assert!(derive_scopes(&source, &config).is_empty());

    let raw = config
        .memory_tree_content_root()
        .join("raw")
        .join("gmail-me");
    std::fs::create_dir_all(&raw).expect("raw dir");
    std::fs::write(raw.join("_source.md"), "scope: \"gmail:me-example-com\"\n")
        .expect("source marker");

    let scopes = derive_scopes(&source, &config);
    assert_eq!(scopes.len(), 1);
    assert_eq!(scopes[0].tree_scope, "gmail:me-example-com");
    assert_eq!(scopes[0].archive_source_id, "gmail:me-example-com");

    // A toolkit with no raw archive convention is not guessed at.
    let notion: MemorySourceEntry = serde_json::from_value(serde_json::json!({
        "id": "notion-src",
        "kind": "composio",
        "label": "Notion",
        "toolkit": "notion",
        "connection_id": "conn-2",
    }))
    .expect("composio source entry");
    assert!(derive_scopes(&notion, &config).is_empty());
}
