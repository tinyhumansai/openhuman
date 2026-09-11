//! `ArchivistHook` struct definition.

use super::boundary::BoundaryConfig;
use crate::openhuman::config::Config;
use crate::openhuman::memory::api::provider::MemoryProvider;
use std::sync::Arc;

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
    ///
    /// Held behind an `Arc` so the hook shares the session factory's single
    /// `Config` snapshot rather than deep-cloning a 95-field struct with
    /// nested `Vec`s into every live agent (openhuman#6218). `Config` is
    /// immutable after construction, so sharing and copying are behaviourally
    /// identical here.
    pub(super) config: Option<std::sync::Arc<Config>>,
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
}
