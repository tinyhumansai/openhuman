//! The session host's memory hooks: the pre-turn pack and the post-turn log.
//!
//! Before the model runs, [`OpenHumanTurnPrelude::memory_pre_turn`] logs the
//! user turn and refreshes memory in the background, taking a completed cached
//! pack immediately (`memory::lifecycle::prefetch::pre_turn`). When the
//! config binds `[memory] recall_each_turn_ms`, it instead waits up to that
//! long for the pack recalled for the current message, alongside the rest of
//! turn setup. `MemoryPackMiddleware` then adds the pack to the turn's
//! requests without persisting it. After the durable
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

use crate::core::runtime::DispatchCtx;
use std::sync::Arc;

use chrono::Utc;
use tinyagents_harness::summarization::is_checkpoint;
use tinyagents_runtime::CommitReceipt;
use tinyinference_llm::message::Message;

use super::OpenHumanTurnPrelude;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::memory::lifecycle::hooks::{self, MemoryTurn, ToolCallSummary};
use crate::memory::lifecycle::prefetch::RecallEachTurn;
use tinymemory_tools::layout::identity::ResolvedIdentity;

/// The memory side of the turn in flight, held from pre-turn to commit.
#[derive(Clone)]
pub(super) struct PendingMemoryTurn {
    identity: ResolvedIdentity,
    user_index: u32,
    channel: Option<String>,
    // The canonical runtime can commit on a different task. Restore the
    // authenticated profile captured at preparation, never infer it from config.
    profile_context: ProfileMemoryContext,
}

/// The provider channel that owns a logged conversation, if this turn came
/// from an external channel. Event-context labels such as `cron`, `voice`, and
/// `mcp_server` are not channel owners.
fn logged_channel(
    origin: Option<&crate::core::runtime::turn_origin::AgentTurnOrigin>,
) -> Option<String> {
    let Some(crate::core::runtime::turn_origin::AgentTurnOrigin::ExternalChannel {
        channel,
        message_id,
        ..
    }) = origin
    else {
        return None;
    };
    if message_id.starts_with("cron:") {
        return None;
    }
    let channel = channel.trim().to_ascii_lowercase();
    if channel.is_empty() {
        return None;
    }
    Some(channel)
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
    /// The context this session's turns run under, from the host environment
    /// bound at the start of the turn.
    /// The connected-identities prompt section, read under the session's context.
    pub(in crate::agent::session_host) fn connected_identities_md(&self) -> String {
        crate::integrations::render_connected_identities(&self.dispatch_ctx())
    }

    pub(in crate::agent::session_host) fn dispatch_ctx(&self) -> crate::core::runtime::DispatchCtx {
        crate::core::runtime::DispatchCtx::from_weak(self.host_env.core.as_ref())
    }
    /// The config memory runs under: the session's own, else the
    /// workspace's on disk.
    pub(super) async fn memory_config(&self) -> Option<Arc<crate::config::Config>> {
        if let Some(config) = &self.runtime_config {
            return Some(config.clone());
        }
        crate::config::rpc::load_config_for_workspace_with_timeout(
            &self.dispatch_ctx(),
            &self.workspace_dir,
        )
        .await
        .map(Arc::new)
        .map_err(|error| {
            log::debug!("[session_host:memory] no config for memory: {error}");
        })
        .ok()
    }

    /// The identity this session's memory runs as: the host environment's
    /// (a team member's), narrowed to this agent.
    pub(super) fn memory_identity(&self, config: &crate::config::Config) -> ResolvedIdentity {
        let acting = self.host_env.memory_identity.clone().unwrap_or_default();
        acting
            .child(&self.agent_definition_id)
            .resolve(&crate::memory::scope::identity_pins(config))
    }

    /// Whether this session may read memory at all: a SaaS turn without an
    /// authenticated profile may not (see `capture_profile_context`).
    pub(super) fn memory_readable(&self) -> bool {
        !matches!(
            capture_profile_context(
                &self.dispatch_ctx(),
                crate::profiles::host::host().as_deref(),
            ),
            ProfileMemoryContext::Denied
        )
    }

    /// Takes completed memory and queues a user-authored turn's log and recall.
    /// Remembers the turn so the reply is logged under the same identity and
    /// index, and returns its memory for the run context without engine I/O.
    /// `origin` is the turn's, as the run context carries it (nothing is read
    /// from the running task); a channel turn is
    /// logged as observed from its sender
    /// ([`crate::memory::lifecycle::sender::channel_actor`]).
    pub(super) async fn memory_pre_turn(
        &self,
        history: &[Message],
        committed_turns: usize,
        current: Option<&Message>,
        origin: Option<crate::core::runtime::turn_origin::AgentTurnOrigin>,
    ) -> Option<Arc<MemoryTurn>> {
        let user_text = self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending_user_text
            .clone()?;
        let thread_id = self.thread_id.clone()?;
        let config = self.memory_config().await?;
        let mut identity = self.memory_identity(&config);
        if self.omit_memory_context {
            identity.recall = false;
        }
        let profile_context = capture_profile_context(
            &self.dispatch_ctx(),
            crate::profiles::host::host().as_deref(),
        );
        if matches!(profile_context, ProfileMemoryContext::Denied) {
            return None;
        }
        let user_index = hooks::user_turn_index(committed_turns);
        let (in_prompt_from, compacted) = in_prompt_window(history, committed_turns, current);
        let memory_channel = logged_channel(origin.as_ref());
        self.mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending_memory_turn = Some(PendingMemoryTurn {
            identity: identity.clone(),
            user_index,
            channel: memory_channel.clone(),
            profile_context,
        });
        let pack = crate::memory::lifecycle::prefetch::pre_turn(
            &self.dispatch_ctx(),
            config.clone(),
            identity.clone(),
            thread_id.clone(),
            user_index,
            user_text,
            in_prompt_from,
            Utc::now(),
            compacted && committed_turns > 0,
            origin
                .as_ref()
                .and_then(crate::memory::lifecycle::sender::channel_actor),
            memory_channel,
            config
                .memory
                .recall_each_turn_ms
                .map(|ms| RecallEachTurn::new(std::time::Duration::from_millis(ms))),
        )
        .await;
        Some(Arc::new(MemoryTurn {
            config,
            identity,
            thread_id,
            pack,
            in_prompt_from: Arc::new(tinymemory_tools::lifecycle::window::InPromptFrom::new(
                in_prompt_from,
                user_index,
            )),
            core: self.host_env.core.clone(),
        }))
    }

    /// Logs the committed reply (spawned; never delays the commit), queues a
    /// plan that finished this turn's notes as learnings, and publishes
    /// `ConversationTurnCommitted`.
    pub(super) fn memory_post_turn(
        &self,
        receipt: &CommitReceipt<OpenHumanRunContext>,
        finished_plan: Option<crate::agent::tinyagents::state_block::FinishedPlan>,
    ) {
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
            let dispatch = prelude.dispatch_ctx();
            hooks::post_turn(
                &dispatch,
                &config,
                &pending.identity,
                thread_id,
                pending.user_index.saturating_add(1),
                assistant_text,
                tool_calls,
                Utc::now(),
                pending.channel,
            )
            .await;
            if let Some(finished) = finished_plan {
                crate::memory::lifecycle::plan_notes::learn_plan_notes(
                    config.clone(),
                    &pending.identity,
                    Some(prelude.action_dir.display().to_string()),
                    &finished.plan,
                    finished.goal.as_deref(),
                )
                .await;
            }
        };
        self.dispatch_ctx()
            .spawn(deferred_memory_work(profile_context, work));
    }
}

// Resolve the profile owner at preparation; a canonical commit may run after
// the immutable turn-origin scope has ended.
fn capture_profile_context(
    ctx: &DispatchCtx,
    host: Option<&crate::profiles::ProfileHost>,
) -> ProfileMemoryContext {
    let Some(context) = ctx.core().cloned() else {
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
#[path = "session_hooks_tests.rs"]
mod tests;
