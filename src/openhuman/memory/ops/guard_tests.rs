//! Tests for [`super::active_memory_guard`].
//!
//! The bypass allowlist ratchet lives in
//! [`crate::openhuman::memory::bypass_allowlist_tests`] — see the note at the
//! foot of this file.

use super::*;

/// The pre-boot fallback resolves *the same* workspace the global client is
/// bound to, not whatever `Config::load_or_init` reports. That is the property
/// the four re-pointed handlers rest on: their existing tests bind a temp
/// workspace through `shared_memory_test_workspace()` and never build a
/// `CoreContext`, so a config-derived fallback would silently guard a
/// different store.
#[tokio::test]
async fn falls_back_to_the_configured_workspace_when_there_is_no_context() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    // The process-global engine slot is gone with the second in-process engine,
    // so there is no `active_workspace_dir()` to interrogate. The fixture's own
    // workspace is the anchor now, and the assertion below — that the fallback
    // resolves to the same binding — is what this test was really about.
    let workspace = crate::openhuman::memory::ops::shared_memory_test_workspace();

    let guard = active_memory_guard().await.expect("guard resolves");
    let bound = binding::for_workspace(&workspace, &MemorySubsystemConfig::default())
        .expect("binding resolves");

    // Same cached binding ⇒ same driver ⇒ same store.
    assert!(
        Arc::ptr_eq(&guard, &bound.guard()),
        "the fallback must reuse the binding cached for that workspace"
    );
}

/// The guard advertises the driver underneath it, not a synthetic id — so a
/// handler routed through it still reports the embedded driver in status and
/// spans.
///
/// The id is read off the bound driver rather than compared against
/// `binding::MODULE_ID`. Spelling the constant here asserted two things at
/// once — that the guard is transparent, and that the fixture happens to bind
/// a driver calling itself `tinymemory` — and only the first is what this test
/// is named for. The second held by luck: the engine this fixture used to bind
/// reported `MODULE_ID` as its own id, so a guard that returned a synthetic
/// `MODULE_ID` and one that passed the driver's id through were
/// indistinguishable. Reading it from the driver tells them apart, and keeps
/// the test honest under any fixture.
#[tokio::test]
async fn guards_the_module_driver_and_keeps_its_identity() {
    use crate::openhuman::memory::api::provider::MemoryProvider;

    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    let workspace = crate::openhuman::memory::ops::shared_memory_test_workspace();
    let bound = binding::for_workspace(&workspace, &MemorySubsystemConfig::default())
        .expect("binding resolves");
    let embedded = bound.unguarded_provider().driver_id().to_string();

    let guard = active_memory_guard().await.expect("guard resolves");
    assert_eq!(
        guard.driver_id(),
        embedded,
        "the guard must report the driver underneath it, not an id of its own"
    );
    assert!(guard.as_documents().is_some());
    assert!(guard.as_graph().is_some());
    assert!(guard.as_tool_memory().is_some());
}

// ── Where the allowlist drift guard lives ────────────────────────────────────
//
// M4b shipped a provisional file-keyed drift guard here. M4c replaced it with a
// `(file, pattern)`-keyed lint in `memory::bypass_allowlist_tests`, which covers
// a strict superset of the call shapes (adding direct driver/engine construction
// and raw `MemoryBinding` reach-through) and reports which needle tripped rather
// than only which file.
//
// It was deleted rather than kept alongside: two allowlists over the same tree
// must both be struck when a bypass is cleaned up, and the one nobody remembers
// is exactly the dead-string rot the ratchet exists to prevent. One list.
