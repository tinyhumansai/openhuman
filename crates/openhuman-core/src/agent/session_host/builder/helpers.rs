//! Utility helpers used during agent construction.
//!
//! # A note on the store this reaches (#5560)
//!
//! [`tool_memory_store`] is host-side now — see
//! [`memory::tool_memory::store`](crate::memory::tool_memory::store)
//! — where it used to be `tinycortex`'s. No call site changed, because the
//! convention it applies (the `tool-<name>` namespace, the `rule/<id>` key, the
//! record's serde) comes from the contract that both ends read.
//!
//! **Do not "modernise" this onto `active_memory_guard().as_tool_memory()`.**
//! `memory` is the session's canonical workspace store. The binding below is
//! shared with the session's `DriverMemory`, so recall and tool-memory rules
//! always address the same durable data.

use crate::memory::api::provider::MemoryProvider;
use crate::memory::auto_recall::AutoRecall;
use crate::memory::tool_memory::{tool_memory_store, ToolMemoryRule};
use crate::memory::Memory;
use std::sync::Arc;

use crate::agent::prompts::SystemPromptBuilder;
use std::collections::HashSet;
use tinytools::Tool;

/// Binds this session's memory subtree once and hands back the two handles
/// the factory takes from it: the raw provider the archivist writes through,
/// and Lane C (#6040) over the same binding's guard, so the auto-recall reads
/// exactly the subtree the session chats against — a dedicated profile recalls
/// its own facts, never the shared tree's.
pub(super) fn bind_session_memory(
    config: &crate::config::Config,
) -> anyhow::Result<(Arc<dyn MemoryProvider>, Arc<AutoRecall>)> {
    let binding = crate::memory::binding::for_config(config)
        .map_err(|e| anyhow::anyhow!("archivist memory binding: {e}"))?;
    let auto_recall = Arc::new(AutoRecall::from_guard(binding.guard()));
    Ok((binding.provider().clone(), auto_recall))
}

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

/// Register the memory prompt sections a session should carry.
///
/// One gate per direction, and neither consults `learning.enabled`: the
/// instructions are about the memory *tools*, which exist whether or not the
/// learning subsystem runs (#6040 untied the read side, #6048 the write side).
/// A model holding those tools with no rule on when to use them is what
/// produced both bugs — claiming absence unsearched, and "got it, saved" with
/// zero tool calls.
///
/// - `MemoryAccessSection` (read side, #566/#6040): a retrieval tool is offered.
/// - `MemoryWriteSection` (write side, #6048): a writing tool is offered, and it
///   names only the write tools this session actually holds.
///
/// Each gate needs the tool registered **and** visible after filtering
/// (`any_tool_offered`): a rule about a tool the model cannot see would only
/// teach it to apologise.
pub(super) fn add_memory_prompt_sections(
    prompt_builder: SystemPromptBuilder,
    tools: &[Box<dyn Tool>],
    delegation_tools: &[Box<dyn Tool>],
    visible: &HashSet<String>,
    agent_id: &str,
) -> SystemPromptBuilder {
    use crate::agent::learning::{
        any_tool_offered, MemoryAccessSection, MemoryWriteSection, MEMORY_READ_TOOLS,
        MEMORY_STORE_TOOL, MEMORY_TOOL, MEMORY_WRITE_DELEGATE_TOOL, SAVE_PREFERENCE_TOOL,
    };
    let mut prompt_builder = prompt_builder;
    // Gate on the set the model will actually see: packs are stripped from
    // `visible` later in the build, and gating on the pre-strip set told the
    // orchestrator to call `save_preference` while the pack held it off the
    // wire.
    let visible = &{
        let mut after = visible.clone();
        crate::tools::toolpacks::strip_packed_from_visible(&mut after, agent_id);
        after
    };
    // The collapsed `memory` tool counts only for the actions it still
    // advertises: a driver that cannot store drops `learn`, and one that cannot
    // search leaves only writes, so the tool's name alone proves neither.
    let memory_actions: Vec<String> = tools
        .iter()
        .chain(delegation_tools)
        .find(|tool| tool.name() == MEMORY_TOOL)
        .filter(|_| any_tool_offered(&[MEMORY_TOOL], tools, delegation_tools, visible))
        .map(|tool| crate::memory::tools::advertised_actions(tool.as_ref()))
        .unwrap_or_default();
    let memory_can_read = memory_actions
        .iter()
        .any(|a| a == "ask" || a == "keyword_search");
    let memory_can_learn = memory_actions.iter().any(|a| a == "learn");
    let legacy_read: Vec<&str> = MEMORY_READ_TOOLS
        .iter()
        .copied()
        .filter(|name| *name != MEMORY_TOOL)
        .collect();
    if memory_can_read || any_tool_offered(&legacy_read, tools, delegation_tools, visible) {
        prompt_builder = prompt_builder.add_section(Box::new(MemoryAccessSection));
        log::debug!("[memory_access] prompt section registered");
    } else {
        log::debug!(
            "[memory_access] skipping MemoryAccessSection — neither memory_recall nor \
             memory_search is registered+visible for agent={agent_id}"
        );
    }
    // Asked per tool, not once for the pair: the section names the routes it
    // is given, so a profile carrying only one write tool must not be told
    // about the other (review finding).
    let preferences = any_tool_offered(&[SAVE_PREFERENCE_TOOL], tools, delegation_tools, visible);
    let store_offered = any_tool_offered(&[MEMORY_STORE_TOOL], tools, delegation_tools, visible);
    // The collapsed `memory` tool writes through its `learn` action; the legacy
    // `memory_store` wins the wording only when both are held.
    let memory_offered = memory_can_learn;
    let facts = store_offered || memory_offered;
    // #6200: asked for as well as the pair, not instead of it. An agent whose
    // only write path is the delegate held the tool and no rule about using it
    // — the write-side twin of the read-side gap #6183 closed.
    let delegate = any_tool_offered(
        &[MEMORY_WRITE_DELEGATE_TOOL],
        tools,
        delegation_tools,
        visible,
    );
    if preferences || facts || delegate {
        let mut section = MemoryWriteSection::new(preferences, facts, delegate);
        if memory_offered && !store_offered {
            section = section.via_memory_tool();
        }
        prompt_builder = prompt_builder.add_section(Box::new(section));
        log::debug!(
            "[memory_write] prompt section registered for agent={agent_id} \
             save_preference={preferences} memory_store={facts} \
             manage_profile_memory={delegate}"
        );
    } else {
        log::debug!(
            "[memory_write] skipping MemoryWriteSection — none of memory_store, \
             save_preference or manage_profile_memory is registered+visible for \
             agent={agent_id}"
        );
    }
    prompt_builder
}

pub(super) fn derive_turn_workspace_descriptor() -> Option<tinytools::WorkspaceDescriptor> {
    let root = crate::agent::turn_workspace::current()?;
    if !root.is_dir() {
        tracing::warn!(
            root = %root.display(),
            "[turn_workspace] scoped root is not an existing directory — \
             falling back to the shared action_dir cwd for this turn"
        );
        return None;
    }
    tracing::debug!(
        root = %root.display(),
        "[turn_workspace] turn bound to the embedder's per-turn root as default cwd"
    );
    Some(tinytools::WorkspaceDescriptor::new(root).with_policy_id("turn-workspace"))
}
