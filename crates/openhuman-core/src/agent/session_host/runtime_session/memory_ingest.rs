//! The session host's memory hooks: the pre-turn pack and the post-turn log.
//!
//! Before the model runs, [`OpenHumanTurnPrelude::memory_pre_turn`] logs the
//! user turn and refreshes memory in the background, taking a completed cached
//! pack immediately (`memory::lifecycle::prefetch::pre_turn`); `MemoryPackMiddleware` then adds the
//! pack to the turn's requests without persisting it. After the durable
//! commit, [`OpenHumanTurnPrelude::memory_post_turn`] logs the reply with its
//! tool calls. Both run under the session's own config, so a host binding set
//! on a derived context (`[memory] agent_id` / `root`) is honoured, and both
//! resolve the identity the turn runs as (`memory::scope`). Deferred profile
//! commits retain a weak reference to the lease-owned root context, so ending
//! a temporary turn scope does not lose the engine binding or pin an idle
//! profile. A retired root cannot fall back to a configured external engine.
//!
//! The committed turn is also published as
//! `DomainEvent::ConversationTurnCommitted` for observers.

use std::sync::Arc;

use chrono::Utc;
use tinyagents_harness::summarization::is_checkpoint;
use tinyagents_runtime::CommitReceipt;
use tinyinference_llm::message::Message;

use super::OpenHumanTurnPrelude;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::memory::lifecycle::hooks::{
    self, MemoryTurn, PostTurnInput, PreTurnInput, ToolCallSummary,
};
use crate::memory::scope::ResolvedIdentity;

/// The memory side of the turn in flight, held from pre-turn to commit.
#[derive(Clone)]
pub(super) struct PendingMemoryTurn {
    identity: ResolvedIdentity,
    user_index: u32,
    // The canonical runtime can commit on a different task. Restore the
    // authenticated profile captured at preparation, never infer it from config.
    profile_context: ProfileMemoryContext,
}

// Missing profile authority must not be confused with an ordinary desktop turn.
#[derive(Clone)]
enum ProfileMemoryContext {
    NonProfile,
    BoundWeak(std::sync::Weak<crate::core::runtime::CoreContext>),
    Denied,
}

/// Where the prompt's verbatim thread starts, as a user turn index, and
/// whether earlier turns were compacted away. Without a checkpoint the whole
/// thread is in the prompt (`0`).
pub(super) fn in_prompt_window(
    history: &[Message],
    committed_turns: usize,
    current: Option<&Message>,
) -> (u32, bool) {
    let Some(checkpoint) = history.iter().rposition(is_checkpoint) else {
        return (0, false);
    };
    let kept_user_turns = history[checkpoint + 1..]
        .iter()
        .filter(|message| matches!(message, Message::User(_)))
        .filter(|message| Some(*message) != current)
        .count();
    (
        hooks::user_turn_index(committed_turns.saturating_sub(kept_user_turns)),
        true,
    )
}

impl OpenHumanTurnPrelude {
    /// The config memory runs under: the session's own, else the
    /// workspace's on disk.
    async fn memory_config(&self) -> Option<Arc<crate::config::Config>> {
        if let Some(config) = &self.runtime_config {
            return Some(config.clone());
        }
        crate::config::rpc::load_config_for_workspace_with_timeout(&self.workspace_dir)
            .await
            .map(Arc::new)
            .map_err(|error| {
                log::debug!("[session_host:memory] no config for memory: {error}");
            })
            .ok()
    }

    /// Takes completed memory and queues a user-authored turn's log and recall.
    /// Remembers the turn so the reply is logged under the same identity and
    /// index, and returns its memory for the run context without engine I/O.
    /// `origin` is the turn's origin as the run context carries it (the
    /// session host never reads the task-local one); a channel turn is
    /// logged as observed from its sender
    /// ([`crate::memory::lifecycle::sender::channel_actor`]).
    pub(super) async fn memory_pre_turn(
        &self,
        history: &[Message],
        committed_turns: usize,
        current: Option<&Message>,
        origin: Option<crate::agent::turn_origin::AgentTurnOrigin>,
    ) -> Option<Arc<MemoryTurn>> {
        let user_text = self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending_user_text
            .clone()?;
        let thread_id = self.thread_id.clone()?;
        let config = self.memory_config().await?;
        let mut identity = crate::memory::scope::resolve_current(&config);
        if self.omit_memory_context {
            identity.recall = false;
        }
        let profile_context = capture_profile_context(crate::profiles::host::host().as_deref());
        if matches!(profile_context, ProfileMemoryContext::Denied) {
            return None;
        }
        let user_index = hooks::user_turn_index(committed_turns);
        let (in_prompt_from, compacted) = in_prompt_window(history, committed_turns, current);
        self.mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending_memory_turn = Some(PendingMemoryTurn {
            identity: identity.clone(),
            user_index,
            profile_context,
        });
        let pack = crate::memory::lifecycle::prefetch::pre_turn(
            config.clone(),
            identity.clone(),
            PreTurnInput {
                thread_id: thread_id.clone(),
                turn_index: user_index,
                user_text,
                in_prompt_from,
                at: Utc::now(),
                resumed_after_compaction: compacted && committed_turns > 0,
                observed_actor: origin
                    .as_ref()
                    .and_then(crate::memory::lifecycle::sender::channel_actor),
            },
            Some(self.event_channel.clone()),
        );
        Some(Arc::new(MemoryTurn {
            config,
            identity,
            thread_id,
            pack,
        }))
    }

    /// Logs the committed reply (spawned; never delays the commit) and
    /// publishes `ConversationTurnCommitted`.
    pub(super) fn memory_post_turn(&self, receipt: &CommitReceipt<OpenHumanRunContext>) {
        let (user_text, pending) = {
            let mut mutable = self
                .mutable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                mutable.pending_user_text.take(),
                mutable.pending_memory_turn.take(),
            )
        };
        let (Some(user_text), Some(thread_id)) = (user_text, self.thread_id.clone()) else {
            return;
        };
        let assistant_text = receipt.outcome.output.clone().unwrap_or_default();
        let tool_calls = committed_tool_calls(&receipt.outcome.history);
        log::debug!(
            "[session_host] conversation turn committed tool_calls={}",
            tool_calls.len()
        );
        crate::core::bus::BUS.publish(
            crate::core::events::DomainEvent::ConversationTurnCommitted {
                thread_id: thread_id.clone(),
                agent_id: Some(self.agent_definition_id.clone()).filter(|id| !id.trim().is_empty()),
                workspace: Some(self.action_dir.display().to_string()),
                channel: Some(self.event_channel.clone())
                    .filter(|channel| !channel.trim().is_empty()),
                user_text,
                assistant_text: assistant_text.clone(),
                tool_calls: tool_calls
                    .iter()
                    .map(|call| crate::core::events::ConversationToolCall {
                        name: call.name.clone(),
                        id: call.id.clone(),
                    })
                    .collect(),
                workspace_dir: self.workspace_dir.clone(),
            },
        );
        let Some(pending) = pending else {
            return;
        };
        let prelude = self.clone();
        let profile_context = pending.profile_context.clone();
        let work = async move {
            let Some(config) = prelude.memory_config().await else {
                return;
            };
            hooks::post_turn(
                &config,
                &pending.identity,
                PostTurnInput {
                    thread_id,
                    turn_index: pending.user_index.saturating_add(1),
                    assistant_text,
                    tool_calls,
                    at: Utc::now(),
                },
            )
            .await;
        };
        crate::core::runtime::spawn_scoped(deferred_memory_work(profile_context, work));
    }
}

// Resolve the profile owner at preparation; a canonical commit may run after
// the immutable turn-origin scope has ended.
fn capture_profile_context(host: Option<&crate::profiles::ProfileHost>) -> ProfileMemoryContext {
    let Some(context) = crate::core::runtime::CoreContext::scoped() else {
        return if crate::core::runtime::is_saas() {
            ProfileMemoryContext::Denied
        } else {
            ProfileMemoryContext::NonProfile
        };
    };
    let Some(profile_id) = context.profile() else {
        return ProfileMemoryContext::NonProfile;
    };
    let Some(profile) = crate::profiles::ProfileId::parse(profile_id)
        .ok()
        .and_then(|id| host.and_then(|host| host.get(&id)))
    else {
        return ProfileMemoryContext::Denied;
    };
    // The host owns this root for the lease's lifetime. A derived turn-origin
    // context ends with the turn, even if its deferred commit is still queued.
    // Keeping only a weak root cannot create an agent/context reference cycle.
    ProfileMemoryContext::BoundWeak(Arc::downgrade(profile.context()))
}

async fn deferred_memory_work(
    profile_context: ProfileMemoryContext,
    work: impl std::future::Future<Output = ()>,
) {
    match profile_context {
        ProfileMemoryContext::BoundWeak(context) => {
            // A retired profile cannot fall back to a configured external engine.
            if let Some(context) = context.upgrade() {
                crate::core::runtime::CoreContext::scope(context, work).await;
            }
        }
        ProfileMemoryContext::NonProfile => work.await,
        ProfileMemoryContext::Denied => {}
    }
}

/// The tool calls of the last exchange in `history` (everything after the
/// final user message), with the start of each result.
fn committed_tool_calls(history: &[Message]) -> Vec<ToolCallSummary> {
    let start = history
        .iter()
        .rposition(|message| matches!(message, Message::User(_)))
        .map_or(0, |index| index + 1);
    let exchange = &history[start..];
    let result_of = |id: &str| {
        exchange.iter().find_map(|message| match message {
            Message::Tool(tool) if !id.is_empty() && tool.tool_call_id == id => {
                Some(message.text())
            }
            _ => None,
        })
    };
    exchange
        .iter()
        .filter_map(|message| match message {
            Message::Assistant(assistant) => Some(&assistant.tool_calls),
            _ => None,
        })
        .flatten()
        .map(|call| ToolCallSummary {
            name: call.name.clone(),
            id: Some(call.id.clone()).filter(|id| !id.is_empty()),
            result: result_of(&call.id),
        })
        .collect()
}

#[cfg(test)]
#[path = "memory_ingest_tests.rs"]
mod tests;
