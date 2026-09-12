//! The host's memory contract must **be** `tinymemory-api`, not a copy of it.
//!
//! `crate::openhuman::memory::api` is the vocabulary three parties speak:
//! the host call sites, [`crate::openhuman::modules::memory::ModuleMemoryProvider`]
//! (which serialises it onto the bus), and the separately compiled TinyMemory
//! module on the far end — which compiles against the `tinymemory-api` **crate**.
//!
//! Commit `3ee5a3cad` ("run tiny domains as TinyBus modules") inlined that
//! crate into `src/openhuman/memory/api/` as 10,894 lines of verbatim copy.
//! Every file was byte-identical to `vendor/tinymemory/api/src/` apart from
//! doc-comment paths, so nothing behaved differently on the day — but the two
//! were now *distinct types* that only a human comparing files could keep in
//! step.
//!
//! That is precisely the failure mode `modules/memory.rs` says the shared error
//! table exists to prevent: "reimplementing the mapping here is what would let
//! a `PathEscape` arrive as an `Invalid`, silently reclassifying a sandbox
//! escape as a caller mistake." `api::wire` is the table, and while the host
//! held its own copy of it that sentence described an *aspiration*, not the
//! build — the host end and the module end were two independent definitions
//! free to drift the moment either side was edited.
//!
//! These assertions are **type identities**, so they fail at compile time
//! rather than at run time. Against the inlined copy this file does not build:
//! `expected 'tinymemory_api::chunks::SourceKind', found
//! 'openhuman::openhuman::memory::api::chunks::SourceKind'`. That is the point
//! — a future re-inlining cannot land quietly, it breaks the build.

/// The chunk vocabulary the tree families pass across the seam.
#[test]
fn chunk_types_are_the_contract_crates() {
    fn source_kind(
        k: crate::openhuman::memory::api::chunks::SourceKind,
    ) -> tinymemory_api::chunks::SourceKind {
        k
    }
    fn chunk(c: crate::openhuman::memory::api::chunks::Chunk) -> tinymemory_api::chunks::Chunk {
        c
    }
    let _ = source_kind as fn(_) -> _;
    let _ = chunk as fn(_) -> _;
}

/// The error enum and the wire table that round-trips it. If these two ever
/// separate, a driver error can be reclassified in transit with nothing failing.
#[test]
fn error_and_wire_table_are_the_contract_crates() {
    fn error(
        e: crate::openhuman::memory::api::error::MemoryError,
    ) -> tinymemory_api::error::MemoryError {
        e
    }
    let _ = error as fn(_) -> _;

    // Same input, same name, from both ends of the seam.
    let host = crate::openhuman::memory::api::wire::wire_name(
        &crate::openhuman::memory::api::error::MemoryError::PathEscape("/etc/passwd".to_string()),
    );
    let crate_side = tinymemory_api::wire::wire_name(
        &tinymemory_api::error::MemoryError::PathEscape("/etc/passwd".to_string()),
    );
    assert_eq!(host, crate_side, "the wire error table must be one table");
}

/// The driver contract itself: a `MemoryProvider` built against the crate must
/// satisfy the host's trait object, or the seam has two provider vocabularies.
#[test]
fn provider_contract_is_the_contract_crates() {
    fn provider(
        p: std::sync::Arc<dyn crate::openhuman::memory::api::provider::MemoryProvider>,
    ) -> std::sync::Arc<dyn tinymemory_api::provider::MemoryProvider> {
        p
    }
    let _ = provider as fn(_) -> _;

    fn capabilities(
        c: crate::openhuman::memory::api::capabilities::Capabilities,
    ) -> tinymemory_api::capabilities::Capabilities {
        c
    }
    let _ = capabilities as fn(_) -> _;
}

/// The **inbound** half of the seam. `memory::api::host` deliberately re-exports
/// named types rather than the whole `tinymemory_api::host` namespace: that
/// namespace is the in-process engine-embedding seam (the persisted
/// `MemoryConfig` sections, `MemoryHostConfig`, `EmbeddingProvider`,
/// `MemoryEventSink`), none of which touches the bus. These do —
/// `modules/memory_host.rs` publishes `MemoryEvent` back onto the host's event
/// bus, answers the module's NLP callback with `SpacyResponse`, and replies to
/// the module's `ComposioHost` calls with the two Composio types — so each is
/// contract, and must be the crate's type rather than a lookalike.
#[test]
fn host_seam_types_are_the_contract_crates() {
    fn event(
        e: crate::openhuman::memory::api::host::MemoryEvent,
    ) -> tinymemory_api::host::MemoryEvent {
        e
    }
    let _ = event as fn(_) -> _;

    fn spacy(
        s: crate::openhuman::memory::api::host::SpacyResponse,
    ) -> tinymemory_api::host::SpacyResponse {
        s
    }
    let _ = spacy as fn(_) -> _;

    fn connection(
        c: crate::openhuman::memory::api::host::ComposioConnection,
    ) -> tinyconnectors_bus::ComposioConnection {
        c
    }
    let _ = connection as fn(_) -> _;

    // Also the type `integrations::composio::types` re-exports, which is what
    // makes the bus reply and the host's own Composio client one type rather
    // than two that serialise the same today.
    fn execute(
        r: crate::openhuman::memory::api::host::ComposioExecuteResponse,
    ) -> crate::openhuman::integrations::composio::types::ComposioExecuteResponse {
        r
    }
    let _ = execute as fn(_) -> _;
}

/// Version negotiation is contract. `memory::binding` and `memory::ops::provider`
/// compare a driver's reported contract against this constant before binding it,
/// so a host copy that drifted from the crate's value would admit a driver the
/// module end considers incompatible — or reject one it does not.
#[test]
fn contract_version_is_the_contract_crates() {
    assert_eq!(
        crate::openhuman::memory::api::CONTRACT_VERSION,
        tinymemory_api::CONTRACT_VERSION,
        "the contract version must be one value, not two"
    );
}

/// `null` is **not** contract surface, and this pins the boundary from the other
/// side. `NullMemoryProvider` is the fallback `memory::binding` installs when no
/// module driver is available — it is what runs when nothing crosses the bus.
/// It is still the crate's type (the host does not get to invent a second
/// provider vocabulary), but it is reached by naming `tinymemory_api::null` at
/// the call site rather than through `memory::api`, which is what keeps "the
/// module contract" and "the host's own use of the crate" distinguishable in
/// the source. If `null` is ever re-added to `memory::api`, delete this test
/// deliberately rather than letting the distinction erode.
#[test]
fn the_fallback_driver_is_the_contract_crates_but_not_contract_surface() {
    fn provider(
        p: std::sync::Arc<tinymemory_api::null::NullMemoryProvider>,
    ) -> std::sync::Arc<dyn crate::openhuman::memory::api::provider::MemoryProvider> {
        p
    }
    let _ = provider as fn(_) -> _;
}
