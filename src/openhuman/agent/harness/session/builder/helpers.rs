//! Utility helpers used during agent construction.
//!
//! # A note on the store this reaches (#5560)
//!
//! [`tool_memory_store`] is host-side now — see
//! [`memory::tool_memory::store`](crate::openhuman::memory::tool_memory::store)
//! — where it used to be `tinycortex`'s. No call site changed, because the
//! convention it applies (the `tool-<name>` namespace, the `rule/<id>` key, the
//! record's serde) comes from the contract that both ends read.
//!
//! **Do not "modernise" this onto `active_memory_guard().as_tool_memory()`.**
//! `memory` here is the session's own subtree — `DriverMemory::for_subtree`,
//! resolved in `factory` from the profile's `memory_subdir` — so a profile with
//! `dedicatedMemory` prefetches its own rules. The ambient guard is the
//! *shared* tree, and routing this through it would quietly merge every
//! profile's tool rules into one prompt. The correct family call is the one
//! reached through this session's own `binding::for_subtree(..)`, which needs
//! the subtree passed in rather than the memory object.

use crate::openhuman::memory::tool_memory::{tool_memory_store, ToolMemoryRule};
use crate::openhuman::memory::Memory;
use std::sync::Arc;

/// (#1400) Best-effort synchronous prefetch of eager tool-scoped rules.
///
/// `from_config_*` is sync but typically runs inside a multi-threaded
/// Tokio runtime (the agent harness path from the channels runtime).
/// We use `block_in_place` + the current runtime handle to call the
/// async store API without restructuring the whole session builder.
///
/// Returns an empty `Vec` (rather than erroring) when:
///   - no Tokio runtime is active (e.g. a sync CLI bootstrap),
///   - the runtime is single-threaded (`block_in_place` would panic),
///   - or the underlying `rules_for_prompt` call returns an error
///     (e.g. the memory backend isn't ready yet).
///
/// Critical / High rules captured later in the session are still
/// available via the `memory_tool_rules_for_prompt` RPC; this prefetch
/// merely seeds the rules that exist at session start.
pub(super) fn prefetch_tool_memory_rules_blocking(
    memory: Arc<dyn Memory>,
    tool_names: &[String],
) -> Vec<ToolMemoryRule> {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return Vec::new();
    };
    if handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::MultiThread {
        return Vec::new();
    }
    let tool_names = tool_names.to_vec();
    tokio::task::block_in_place(|| {
        handle.block_on(async move {
            let store = tool_memory_store(memory);
            match store.rules_for_prompt(&tool_names).await {
                Ok(grouped) => {
                    let mut flat: Vec<_> = grouped.into_values().flatten().collect();
                    flat.sort_by(|a, b| {
                        b.priority
                            .cmp(&a.priority)
                            .then_with(|| a.tool_name.cmp(&b.tool_name))
                            .then_with(|| a.rule.cmp(&b.rule))
                    });
                    flat
                }
                Err(err) => {
                    log::warn!("[memory::tool_memory] prefetch failed: {err}");
                    Vec::new()
                }
            }
        })
    })
}
