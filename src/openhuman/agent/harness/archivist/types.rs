//! `ArchivistHook` struct definition.

use super::boundary::BoundaryConfig;
use crate::openhuman::config::Config;
use crate::openhuman::memory::api::provider::MemoryProvider;
use std::sync::Arc;
// Test-only. See [`ArchivistHook::chat_provider`] for why the engine's chat
// trait is still named here and why it is not named in a production build.
#[cfg(test)]
use tinymemory_core::chat::ChatProvider;

/// Post-turn hook that indexes conversation turns and manages segments.
pub struct ArchivistHook {
    /// The bound memory driver this archivist writes through.
    ///
    /// This used to be the raw SQLite connection shared with `UnifiedMemory`,
    /// which is precisely the handle no remote or module driver can supply —
    /// the `:290` blocker the #5378 correction documented. Every episodic and
    /// profile write now goes through the provider's capability families, so
    /// the archivist works against whatever driver the workspace bound.
    pub(super) provider: Option<Arc<dyn MemoryProvider>>,
    /// Whether the archivist is enabled.
    pub(super) enabled: bool,
    /// Boundary detection configuration.
    pub(super) boundary_config: BoundaryConfig,
    /// Optional runtime config — used to gate the tree-ingest path and to
    /// build the LLM chat provider.
    ///
    /// When `None`, the tree-ingest path is skipped. Set via
    /// [`ArchivistHook::with_config`] on the production path.
    pub(super) config: Option<Config>,
    /// Whether an LLM summariser can be built for this workspace. `false`
    /// means the heuristic bookend summary is used instead.
    ///
    /// This was `chat_provider.is_some()` — the archivist built a chat provider
    /// in [`ArchivistHook::with_config`], stored it, and then never called it.
    /// It could not: the summariser it drives is
    /// `tinymemory_core::tree::summarise::summarise`, which builds
    /// its **own** provider from the same `Config`. The stored handle was a
    /// probe result wearing the shape of a dependency, so it is recorded as
    /// what it always was — a yes/no — and the probe now runs against the
    /// host's own inference factory rather than the memory engine's wrapper
    /// around it (#5560). See `with_config` for why those two answer alike.
    pub(super) summariser_available: bool,
    /// Test-only deterministic chat provider, installed into the engine's chat
    /// task-local for the duration of a summarise or ingest call.
    ///
    /// **This is an override, not a dependency**, which is why it is
    /// `#[cfg(test)]` and why nothing in a production build names
    /// `tinymemory_core::chat`. `summarise` and the driver's `ingest_chat` both
    /// construct their own provider; `tinymemory_core::chat::build_chat_runtime`
    /// consults a task-local before building one, and scoping a call through it
    /// is the only way to keep these tests off the network.
    ///
    /// It cannot be re-pointed at the host's own chat seam
    /// (`modules::memory_host`'s `ChatHost` bus interface): that seam is served
    /// to a **loaded module**, and a module cannot be loaded by these tests at
    /// all — `dlopen` is a process singleton, so a second module-loading test in
    /// one process hangs rather than fails. The driver under test is the
    /// in-process `TinycortexProvider`, which reaches its LLM through
    /// `tinymemory-core`, so the engine's task-local is the seam that exists.
    /// The engine stays a dev-dependency for exactly this kind of fixture.
    #[cfg(test)]
    pub(super) chat_provider: Option<Arc<dyn ChatProvider>>,
}
