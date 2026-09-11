//! The **tinybus module contract** for memory — the vocabulary that crosses the
//! bus between this host and the separately compiled TinyMemory module, named
//! as a re-export of the [`tinymemory_api`] crate and never as a copy of it.
//!
//! # Why this file is nine lines and not ten thousand
//!
//! This module used to be a directory holding 10,894 lines that were, file for
//! file, byte-identical to `vendor/tinymemory/api/src/` apart from the paths
//! written inside doc comments. Commit `3ee5a3cad` ("run tiny domains as
//! TinyBus modules") put them there.
//!
//! Nothing behaved differently the day that landed, which is exactly what made
//! it worth undoing. The contract is the vocabulary **three** parties speak:
//! host call sites, [`crate::openhuman::modules::memory::ModuleMemoryProvider`]
//! serialising onto the bus, and the separately compiled TinyMemory module on
//! the far end — and that module compiles against the crate. A verbatim copy
//! made the host's `MemoryError`, `Chunk`, `Capabilities` and `MemoryProvider`
//! *distinct types* from the ones on the wire, kept in step by nothing but
//! whoever remembered to edit both.
//!
//! [`wire`] is where that mattered most. Its module docs, and
//! `modules/memory.rs`, both say the error table is shared by both ends
//! precisely because reimplementing it "is what would let a `PathEscape` arrive
//! as an `Invalid`, silently reclassifying a sandbox escape as a caller
//! mistake". While the host held its own copy of that table the sentence
//! described an intention rather than the build.
//!
//! # What this module exports, and what it deliberately does not
//!
//! It is the contract surface, **not a convenience alias for the crate**. The
//! set below is derived from what actually crosses the bus, in both directions:
//! outbound from `modules/memory.rs`, where `ModuleMemoryProvider` serialises
//! each capability family onto the wire, and inbound from
//! `modules/memory_host.rs`, the host callbacks the module calls back into.
//!
//! The distinction is the point. `tinymemory-api` is also the crate this host
//! embeds the memory *engine* through, and those two roles are not the same
//! surface. Anything the host uses for its own purposes — engine config
//! sections, the driver-fallback provider, trait scaffolding — is reached by
//! naming [`tinymemory_api`] directly at the call site, so that "the module
//! contract" and "the host's own use of the crate" are told apart in the
//! source rather than in someone's memory.
//!
//! Excluded, with the reason:
//!
//! - **`host`** is re-exported as **two types, not the namespace.** It is the
//!   engine-embedding seam — `MemoryConfig` and the other persisted config
//!   sections, `EmbeddingProvider`, `MemoryHostConfig`, `MemoryEventSink` —
//!   which the host hands to `tinymemory-core` in-process and which never
//!   touch the bus. Only [`host::MemoryEvent`] and [`host::SpacyResponse`]
//!   cross it, inbound, and only those two are exported here.
//! - **`null`** is `NullMemoryProvider`, the fallback bound by
//!   `memory::binding` when no module driver is available. It is what runs
//!   when nothing crosses the bus, which makes it the opposite of contract.
//! - **`traits`** is the engine-embedding scaffolding (`Memory` supertrait and
//!   its provider helpers). It is not wire vocabulary and never crosses a bus
//!   boundary. `memory::mod` re-exports `tinymemory_api::traits::Memory`
//!   directly for the handful of in-process callers that need it; reaching it
//!   through this facade would make engine-only scaffolding appear to be contract
//!   surface. **`version`** (as a module path) and **`is_compatible`** likewise
//!   had no bus use; version negotiation uses [`CONTRACT_VERSION`] directly.
//!
//! Everything else is re-exported as a whole namespace on purpose: each of
//! `capabilities`, `chunks`, `error`, `goals`, `health`, `provider` (with its
//! `provider::types` payload vocabulary), `recall`, `tool_memory`, `tree`,
//! `types` and `wire` is wire vocabulary end to end — `provider` alone is the
//! fourteen capability-family traits plus eleven payload types, and naming
//! those twenty-five individually would restate the crate's own module
//! structure without narrowing anything.
//!
//! `memory/api_identity_tests.rs` pins the survivors with type identities, so a
//! future re-inlining fails to compile instead of passing silently.
//!
//! Prefer naming [`tinymemory_api`] directly in new code. These paths exist so
//! the several hundred existing `memory::api::…` call sites did not have to
//! move in the same change that removed the duplicate.

pub use tinymemory_api::{
    capabilities, chunks, error, goals, health, provider, recall, tool_memory, tree, types, wire,
    CONTRACT_VERSION,
};

/// The inbound half of the seam: the `tinymemory_api::host` types that cross
/// the bus, named one at a time rather than as the whole engine-embedding
/// namespace.
///
/// `modules/memory_host.rs` serves the callbacks two of them belong to —
/// [`MemoryEvent`] is what the module publishes back into the host's event bus,
/// and [`SpacyResponse`] answers the module's NLP callback. [`EvidenceRef`]
/// joined them with the `Profile` capability family: it is not a callback type,
/// but it is a field of `provider::profile::ProfileFacet`, so it crosses the bus
/// in both directions whenever a facet does. [`ComposioConnection`] and
/// [`ComposioExecuteResponse`] joined them with the `ComposioHost` interface:
/// they are what `ListConnections` and `Execute` reply with once Composio is
/// reached over the bus instead of through the in-process engine.
///
/// That is the same rule the rest of this list follows — what actually crosses
/// — and it is why these entries are here rather than being named on the crate
/// at each call site. It is also why `host::composio` is *not* re-exported
/// whole: that namespace carries toolkits, catalog entries, GitHub repos and
/// trigger payloads, none of which this seam moves. The rest of
/// `tinymemory_api::host` is the in-process engine seam and must still be named
/// on the crate.
pub mod host {
    pub use tinyconnectors_bus::{ComposioConnection, ComposioExecuteResponse};
    pub use tinymemory_api::host::{EvidenceRef, MemoryEvent, SpacyResponse};
}
