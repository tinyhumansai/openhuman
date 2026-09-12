//! Tests for the driver-backed handlers in [`super`].
//!
//! A sibling file rather than an inline `mod tests`, because the null-driver
//! fixture below names `NullMemoryProvider` and the memory-guard bypass scanner
//! skips `*_tests.rs` by path but not an inline module. Constructing a null
//! driver to prove a refusal names it is not a production bypass, and the list
//! that scanner guards is one that may shrink and must never grow — so the
//! honest fix is to put the test where the repo already puts tests.

use super::*;
use crate::core::subsystem::DriverClass;
use crate::openhuman::memory::api::error::MemoryError;
// Needed to call the family accessors on the *concrete* null provider
// below; the handlers above reach them through `dyn MemoryProvider`, where
// the trait is in scope by construction.
use crate::openhuman::memory::api::provider::MemoryProvider;
use std::sync::Arc;
use tinymemory_api::null::{NullMemoryProvider, NULL_DRIVER_ID};

/// The refusal always names the bound driver. That is the whole contract of
/// this message: an operator reading it has to be able to tell "no driver
/// serves sync" apart from "the sync failed", and the driver id is what
/// carries the difference.
#[test]
fn the_refusal_names_the_bound_driver_and_the_family() {
    let binding = crate::openhuman::memory::binding::bind_provider_for_test(
        Arc::new(NullMemoryProvider::new()),
        DriverClass::Null,
    );
    let message = unserved(&binding, "source sync", "sync_audit_log");

    assert!(
        message.contains(NULL_DRIVER_ID),
        "the refusal must name the driver, got: {message}"
    );
    assert!(
        message.contains("source sync"),
        "the refusal must name the family, got: {message}"
    );
}

/// The null driver really does serve neither family — the premise every
/// `else` arm in this file rests on. A driver that started serving them
/// would make those arms unreachable, and silently.
#[test]
fn the_null_driver_serves_neither_family() {
    let provider = NullMemoryProvider::new();
    assert!(provider.as_source_sync().is_none());
    assert!(provider.as_coding_sessions().is_none());
}

/// openhuman#5820: the all-in sweep must aggregate per-source trigger
/// failures into the response instead of laundering them — in the incident
/// every source failed `no memory source registered` and the RPC still
/// answered a clean success with `sync_triggered: 0` and nothing else.
#[tokio::test]
async fn trigger_sweep_aggregates_failures_instead_of_laundering_them() {
    fn entry(id: &str, enabled: bool) -> MemorySourceEntry {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "kind": "folder",
            "label": "Test",
            "enabled": enabled,
            "path": ".",
        }))
        .expect("a valid folder source entry")
    }

    let sources = vec![
        entry("ok_1", true),
        entry("src_gone", true),
        entry("disabled", false),
        entry("src_also_gone", true),
    ];

    // Succeeds for ids starting `ok`, refuses everything else the way the
    // incident did.
    let (triggered, errors) = trigger_enabled_syncs(&sources, |source| async move {
        if source.id.starts_with("ok") {
            Ok(())
        } else {
            Err(format!(
                "not found: no memory source registered as {}",
                source.id
            ))
        }
    })
    .await;

    assert_eq!(triggered, 1, "only the healthy enabled source triggers");
    assert_eq!(errors.len(), 2, "each failed enabled source is reported");
    assert!(errors[0].starts_with("src_gone: "), "got: {}", errors[0]);
    assert!(
        errors[0].contains("no memory source registered"),
        "the caller must see the real reason, got: {}",
        errors[0]
    );
    assert!(
        errors[1].starts_with("src_also_gone: "),
        "got: {}",
        errors[1]
    );
}

/// openhuman#6007: the per-row Sync button and the Apply-all sweep must route a
/// row the same way, so the decision is one function and this is its test.
///
/// The bug was not that either call site was wrong in isolation — it was that
/// there were two of them. Apply-all sent Composio rows through
/// `MemorySourceSync`, which the driver refuses for exactly that kind, so "sync
/// everything" failed for every connector source. Asserting the *rule* is what
/// stops the two drifting again; a test that drove a full sync could not, since
/// these handlers read the bound driver and are not reachable from a test.
#[test]
fn composio_rows_dispatch_to_the_connector_and_everything_else_to_the_driver() {
    fn composio(id: &str, connection_id: Option<&str>) -> MemorySourceEntry {
        let mut entry: MemorySourceEntry = serde_json::from_value(serde_json::json!({
            "id": id,
            "kind": "composio",
            "label": "Gmail",
            "enabled": true,
            "toolkit": "gmail",
            "connection_id": connection_id,
        }))
        .expect("a valid composio source entry");
        // Set through the struct rather than the JSON so this test does not
        // depend on `max_items` being an accepted input field for the kind.
        entry.max_items = Some(25);
        entry
    }

    fn folder(id: &str) -> MemorySourceEntry {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "kind": "folder",
            "label": "Notes",
            "enabled": true,
            "path": ".",
        }))
        .expect("a valid folder source entry")
    }

    assert_eq!(
        sync_dispatch(&composio("src_gmail", Some("ca_1"))).expect("a connected composio row"),
        SyncDispatch::Connector {
            connection_id: "ca_1".to_string(),
            max_items: Some(25),
        },
        "a Composio row must route to the connector, carrying its connection and cap"
    );

    assert_eq!(
        sync_dispatch(&folder("src_notes")).expect("a folder row"),
        SyncDispatch::Driver,
        "every non-Composio kind stays on the driver's own pipeline"
    );

    // A connection-less Composio row is malformed, not retryable: the connector
    // call is addressed by connection, so there is nothing to sync.
    let error = sync_dispatch(&composio("src_broken", None))
        .expect_err("a composio row with no connection cannot be dispatched");
    assert!(
        error.contains("src_broken") && error.contains("remove and re-add"),
        "the refusal must name the row and say what to do about it, got: {error}"
    );
}

// ── what `add_rpc` owns ─────────────────────────────────────────────────────
//
// The handler generates the source id, maps the request into a
// `MemorySourceEntry`, and applies conservative per-kind caps before the
// registry sees it. Only the first two are its own code; the third is
// `tinymemory-sources`' `apply_kind_defaults`, and what belongs here is that
// this handler *calls* it — an add whose caps the user left unset must not
// reach the registry uncapped.
//
// Covered only by `tests/raw_coverage/memory_threads_raw_coverage_e2e.rs`
// before now, which went with the engine (#6161) although none of this is the
// engine's (#6172).

/// Pins `OPENHUMAN_WORKSPACE` for the duration, holding the crate's env lock so
/// concurrent tests cannot observe the change. `add_rpc` writes through
/// `registry::add_source`, which resolves its config with
/// `load_config_with_timeout` — process-global, so the workspace has to be
/// pinned rather than passed.
struct WorkspaceEnvGuard {
    _env_lock: std::sync::MutexGuard<'static, ()>,
    previous: Option<std::ffi::OsString>,
}

impl WorkspaceEnvGuard {
    fn pin(workspace: &std::path::Path) -> Self {
        let env_lock = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
        std::env::set_var("OPENHUMAN_WORKSPACE", workspace);
        Self {
            _env_lock: env_lock,
            previous,
        }
    }
}

impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var("OPENHUMAN_WORKSPACE", value),
            None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
        }
    }
}

fn github_add_request() -> AddRequest {
    AddRequest {
        kind: tinymemory_sources::types::SourceKind::GithubRepo,
        label: "A repository".into(),
        enabled: true,
        toolkit: None,
        connection_id: None,
        path: None,
        glob: None,
        url: Some("https://github.invalid/owner/repo".into()),
        branch: None,
        paths: Vec::new(),
        // The three the caller left unset, which is the whole point.
        max_commits: None,
        max_issues: None,
        max_prs: None,
        query: None,
        since_days: None,
        max_items: None,
        selector: None,
        max_tokens_per_sync: None,
        max_cost_per_sync_usd: None,
        sync_depth_days: None,
    }
}

#[tokio::test]
async fn add_generates_an_id_and_caps_a_request_that_left_its_limits_unset() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let _env = WorkspaceEnvGuard::pin(&workspace);

    let added = add_rpc(github_add_request())
        .await
        .expect("add_rpc")
        .value
        .source;

    // ── the id is the handler's ─────────────────────────────────────────────
    //
    // The caller never supplies one; a request that could name its own id would
    // let two sources collide by construction.
    let minted = added.id.strip_prefix("src_").unwrap_or_else(|| {
        panic!(
            "the handler must mint a `src_`-prefixed id, got {:?}",
            added.id
        )
    });
    assert!(
        minted.len() == 32
            && minted
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "the suffix must be a uuid-simple — 32 lowercase hex digits — and not \
         merely 32 characters, got {minted:?}"
    );

    // ── the caps came from somewhere ────────────────────────────────────────
    //
    // Asserted as "no longer None" plus the concrete GitHub values, because the
    // two say different things: the first is that this handler applies defaults
    // at all, the second that it applied *these* — a handler that filled them
    // with zeros would satisfy the first and cap every sync at nothing.
    assert_eq!(added.max_prs, Some(10), "per-kind PR cap");
    assert_eq!(added.max_issues, Some(10), "per-kind issue cap");
    assert_eq!(added.max_commits, Some(50), "per-kind commit cap");

    // ── and it reached the registry ─────────────────────────────────────────
    //
    // The response alone would be satisfied by a handler that shaped an entry
    // and dropped it.
    let fetched = get_rpc(GetRequest {
        id: added.id.clone(),
    })
    .await
    .expect("get_rpc")
    .value
    .source;
    let fetched = fetched.expect("the added source is not readable back from the registry");
    assert_eq!(
        fetched.id, added.id,
        "the registry read back a different source"
    );

    // The id alone would be satisfied by a registry that persisted an entry
    // with every other field defaulted. These four are the request's
    // non-default fields, so each one is a mapping the handler had to carry.
    assert_eq!(
        fetched.kind,
        tinymemory_sources::types::SourceKind::GithubRepo,
        "the request's kind must survive the round trip"
    );
    assert_eq!(fetched.label, "A repository", "the request's label");
    assert!(fetched.enabled, "the request asked for an enabled source");
    assert_eq!(
        fetched.url.as_deref(),
        Some("https://github.invalid/owner/repo"),
        "the request's url"
    );
}

// ── Profile-drift diagnosis on sync (#5820 follow-up) ────────────────────────
//
// The memory module is loaded once per process and tinybus never unloads a
// library, so the `config_path` it is handed at load is the one it keeps. When
// the host rebinds its workspace mid-process — boot signed out, then log in —
// the host starts writing `[[memory_sources]]` into the new profile's
// config.toml while the module is still reading the old one. Every id the host
// registers is then unknown to the driver, and the user sees a bare
// `NotFound` that reads like a missing source rather than a stale binding.
//
// The host holds both halves of that contradiction at the point of failure: its
// own registry has the id, and the driver says it does not. Saying so is the
// difference between a ten-minute diagnosis and half a day.

#[test]
fn a_not_found_for_a_source_the_host_has_registered_reports_the_stale_binding() {
    let message = describe_source_sync_failure(
        "src_8cc5c81a3a7b482b8f832a84faaa6c4e",
        true,
        &MemoryError::NotFound(
            "no memory source registered as src_8cc5c81a3a7b482b8f832a84faaa6c4e".to_string(),
        ),
    );
    assert!(
        message.contains("different profile") || message.contains("restart"),
        "a NotFound for an id the host DID register must name the stale binding and the \
         remedy, not repeat the driver's 'no memory source registered' — got: {message}"
    );
    assert!(
        message.contains("src_8cc5c81a3a7b482b8f832a84faaa6c4e"),
        "the message must still name the source id: {message}"
    );
}

#[test]
fn a_not_found_for_an_id_the_host_never_registered_is_passed_through() {
    let message = describe_source_sync_failure(
        "src_never_existed",
        false,
        &MemoryError::NotFound("no memory source registered as src_never_existed".to_string()),
    );
    assert!(
        !message.contains("different profile"),
        "an id the host does not have either is a genuinely missing source, not drift — \
         diagnosing it as a stale binding would send the reader down the wrong path: {message}"
    );
}

#[test]
fn a_non_not_found_failure_is_passed_through_unchanged() {
    let inner = MemoryError::Backend("sqlite is locked".to_string());
    let message = describe_source_sync_failure("src_1", true, &inner);
    assert_eq!(
        message,
        inner.to_string(),
        "only NotFound is ambiguous between 'missing' and 'stale binding'; every other \
         failure must reach the caller as the driver stated it"
    );
}
