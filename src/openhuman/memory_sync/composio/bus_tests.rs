//! Unit tests for the composio connection-created event handler's gating.

use super::toolkit_is_memory_source_registrable;
use crate::openhuman::memory_sync::composio::init_default_composio_sync_providers;

/// #4957 regression: the connection-created handler must only auto-register a
/// toolkit as a memory source when a native memory-sync provider exists for it.
/// This locks the skip decision (`toolkit_is_memory_source_registrable`) that
/// both auto-register sites in `handle` consult — a toolkit with no provider
/// (the prod offenders `googlecalendar` / `googlesheets`) has no
/// `build_pipeline` arm and must be skipped, never becoming a memory source
/// that reports ACTIVE and then silently fails every sync.
#[test]
fn only_provider_backed_toolkits_are_memory_source_registrable() {
    init_default_composio_sync_providers();

    // Built-in providers exist → registrable.
    assert!(toolkit_is_memory_source_registrable("gmail"));
    assert!(toolkit_is_memory_source_registrable("slack"));
    assert!(toolkit_is_memory_source_registrable("github"));

    // No provider → skipped (the exact #4957 failures the human hit in prod).
    assert!(!toolkit_is_memory_source_registrable("googlecalendar"));
    assert!(!toolkit_is_memory_source_registrable("googlesheets"));
    // Unknown / empty slugs are likewise not registrable.
    assert!(!toolkit_is_memory_source_registrable(
        "definitely-not-a-toolkit"
    ));
    assert!(!toolkit_is_memory_source_registrable(""));
}
