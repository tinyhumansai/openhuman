//! Host source for the harness `MemoryPackMiddleware`.
//!
//! The session host recalls one pack per turn before the model runs
//! (`memory::lifecycle::hooks::pre_turn`) and parks it on the run context
//! (`OpenHumanRunContext::memory_turn`). The harness middleware adds
//! [`TurnMemoryPack`]'s text to every model request of that turn as an
//! ephemeral instruction, so the pack is never committed to the thread's
//! history or the provider's cached prefix.

use tinyagents_harness::middleware::MemoryPackSource;

use crate::agent::tinyagents::host::OpenHumanRunContext;

/// Reads the turn's memory pack off the OpenHuman run context.
pub(crate) struct TurnMemoryPack;

impl MemoryPackSource<OpenHumanRunContext> for TurnMemoryPack {
    fn injection(&self, data: &OpenHumanRunContext) -> Option<String> {
        let pack = data.memory_turn.as_ref()?.pack.as_ref()?;
        tracing::trace!(
            tokens = pack.tokens,
            refs = pack.refs.len(),
            "[tinyagents::mw] memory pack added to the model request"
        );
        Some(pack.injection())
    }
}

/// The harness middleware over [`TurnMemoryPack`].
pub(crate) fn memory_pack_middleware(
) -> tinyagents_harness::middleware::MemoryPackMiddleware<OpenHumanRunContext, TurnMemoryPack> {
    tinyagents_harness::middleware::MemoryPackMiddleware::new(TurnMemoryPack)
}

#[cfg(test)]
#[path = "memory_pack_tests.rs"]
mod tests;
