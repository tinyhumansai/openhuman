//! Tool: `agent_prepare_context` — "plan mode as a subagent".
//!
//! When a parent agent explicitly needs an ad hoc context pass, it can call
//! `agent_prepare_context`. This runs the read-only `context_scout` sub-agent
//! inline (blocking), which gathers context from memory, the user's
//! goals/profile, connected integrations, and the web, then returns a tight
//! `[context_bundle]` envelope: whether there's enough context to act, a
//! compact context summary, and an ordered set of recommended next tool calls
//! drawn from the *parent's own* tool catalogue.
//!
//! The scout's output is bounded by `context_scout`'s `max_result_chars`
//! (≈1000 tokens) so the parent's context only grows by a bounded amount.

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use crate::openhuman::agent::harness::fork_context::{
    current_agent_context_prepared_sources, current_parent, AgentContextPreparedSource,
};
use crate::openhuman::agent::harness::subagent_runner::{
    run_subagent, SubagentRunError, SubagentRunOptions, SubagentRunStatus,
};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent::tinyagents::thread_context::current_thread_id;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCallOptions, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::fmt::Write as _;
use tinyagents::harness::tool::ToolExecutionContext;
use tinyagents::harness::workspace::WorkspaceDescriptor;

/// The sub-agent archetype this tool drives.
const SCOUT_AGENT_ID: &str = "context_scout";

/// Extract exactly one `[context_bundle] … [/context_bundle]` envelope from
/// `output`, tolerating surrounding prose, and return only the envelope
/// substring (tags included). Returns `None` when no usable envelope is
/// present.
///
/// The harness prepends every non-error `run_context_scout` result to turn 1 as
/// "Prepared context", and the `context_scout` contract is "emit the single
/// envelope and nothing outside it". But the scout runs on a fast chat-tier
/// model that regularly wraps the envelope in a preamble (`Sure, here's what I
/// found:\n[context_bundle]…`) or a closing line (`…[/context_bundle]\nHope
/// that helps!`). Requiring the *whole* trimmed output to be the envelope made
/// any such wrap fail validation, so the harness silently dropped an
/// otherwise-good bundle and injected nothing — the "scout runs, bundle
/// missing" failure.
///
/// Pulling the envelope substring out of the surrounding text keeps the safety
/// property intact: we still never inject the model's free-form prose, only the
/// bracketed envelope itself. We still reject genuinely unusable output —
/// absent, unterminated/reversed, or duplicated (where we can't tell which
/// envelope is authoritative) — by returning `None`, so the caller falls back
/// to the un-augmented message.
fn extract_context_bundle(output: &str) -> Option<String> {
    const OPEN: &str = "[context_bundle]";
    const CLOSE: &str = "[/context_bundle]";
    // Exactly one open + one close tag. Duplicates are a contract violation we
    // reject rather than guess which envelope is authoritative.
    if output.matches(OPEN).count() != 1 || output.matches(CLOSE).count() != 1 {
        return None;
    }
    let open_idx = output.find(OPEN)?;
    let close_idx = output.find(CLOSE)?;
    // Tags must appear in order (open before close) and not overlap.
    if close_idx < open_idx + OPEN.len() {
        return None;
    }
    let end = close_idx + CLOSE.len();
    Some(output[open_idx..end].trim().to_string())
}

fn already_prepared_context_bundle(sources: &[AgentContextPreparedSource]) -> String {
    let source_names = if sources.is_empty() {
        "the OpenHuman harness".to_string()
    } else {
        sources
            .iter()
            .map(|source| source.source.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let has_enough_context = if sources
        .iter()
        .any(|source| source.has_enough_context == Some(false))
    {
        false
    } else {
        sources
            .iter()
            .any(|source| source.has_enough_context == Some(true))
    };
    let sufficiency_note = if has_enough_context {
        "The earlier prepared context reported enough context."
    } else {
        "This no-op result does not assert that enough context is available; \
         inspect the earlier prepared-context blocks for sufficiency and recommended follow-up tools."
    };
    format!(
        "[context_bundle]\nhas_enough_context: {has_enough_context}\nproposed_goal: none\n\
         summary: Agent context has already been prepared once for this turn by {source_names}. \
         {sufficiency_note} Use the existing prepared-context blocks in the current user message; do not run \
         another context_scout pass.\nrecommended_tool_calls:\n[/context_bundle]"
    )
}

/// Run the `context_scout` sub-agent inline (blocking) for `question` and
/// return its bounded `[context_bundle]` envelope as a [`ToolResult`].
///
/// This is the shared engine behind two callers:
///
/// 1. The [`AgentPrepareContextTool`] — invoked *autonomously by the LLM*
///    when it decides to scout context mid-turn.
/// 2. The agent harness itself — when "super context" is enabled it calls
///    this directly on the first turn of a new thread (see
///    [`crate::openhuman::config::ContextConfig::super_context_enabled`]),
///    so the collection happens regardless of the model's decision.
///
/// Must be called from within an active agent turn (i.e. with the
/// [`crate::openhuman::agent::harness::fork_context::PARENT_CONTEXT`]
/// task-local installed) — it reads the parent's visible tool catalogue
/// and runs the scout against the parent's provider. Outside a turn the
/// `run_subagent` call surfaces a no-parent error as a [`ToolResult::error`].
pub async fn run_context_scout(question: &str, focus: Option<&str>) -> anyhow::Result<ToolResult> {
    let tool_catalog = AgentPrepareContextTool::render_parent_tool_catalog();
    run_context_scout_with_catalog(question, focus, &tool_catalog).await
}

/// Same as [`run_context_scout`] but with an **explicitly-supplied** tool
/// catalogue — for callers *outside* an agent turn that can't auto-derive the
/// parent's visible tool set from `current_parent()` (e.g. the subconscious
/// engine's structured tick).
///
/// The caller passes the catalogue of tools the eventual decision agent can
/// actually call (one `- name: description` per line), so the bundle's
/// `recommended_tool_calls` stay grounded in callable tools.
///
/// **A parent execution context is still required.** Like [`run_context_scout`]
/// this spawns `context_scout` via `run_subagent`, which resolves its provider /
/// tools / model from the `PARENT_CONTEXT` task-local and returns
/// `NoParentContext` when it is unset. A background surface with no enclosing
/// turn MUST establish a root parent first — call this *inside*
/// [`with_root_parent`](crate::openhuman::agent::orchestration::parent_context::with_root_parent).
/// Skipping that is exactly the TAURI-RUST-HMW failure (#4337): every spawn
/// died with `NoParentContext` and the tick ran un-grounded. Only the
/// progress / subagent-lifecycle telemetry degrades gracefully without a
/// parent — `parent_session` falls back to `standalone` and the absent progress
/// sink no-ops — but the spawn itself does not.
pub async fn run_context_scout_with_catalog(
    question: &str,
    focus: Option<&str>,
    tool_catalog: &str,
) -> anyhow::Result<ToolResult> {
    run_context_scout_with_catalog_and_workspace(question, focus, tool_catalog, None).await
}

/// The text [`log_scout_failure`] classifies: a failed run flattened into one
/// string, `source` chain included.
///
/// The billing classifiers key on the **provider wire body**
/// (`… "errorCode":"USER_INSUFFICIENT_CREDITS"`), which arrives inside
/// [`SubagentRunError::Provider`]'s `anyhow` error — often as a *cause* under a
/// context layer, so `to_string()` alone renders only `"provider call failed:
/// <top-level context>"` and would miss the very failure this exists to demote.
/// Walk `anyhow`'s chain for that variant; every other variant is a flat
/// `thiserror` message already. Used for classification only — the
/// user-visible `message` stays `err.to_string()`, unchanged.
fn scout_failure_signal(err: &SubagentRunError) -> String {
    let SubagentRunError::Provider(inner) = err else {
        return err.to_string();
    };
    let mut chain = inner.to_string();
    for cause in inner.chain().skip(1) {
        chain.push_str(": ");
        chain.push_str(&cause.to_string());
    }
    chain
}

/// Is a failed `context_scout` run just the user being out of credits?
///
/// Two billing shapes reach here, both **user-state, not defects**:
/// * the managed OpenHuman backend's budget-exhausted 400
///   (`{"error":"Insufficient budget","errorCode":"USER_INSUFFICIENT_CREDITS"}`),
///   matched by [`crate::openhuman::inference::provider::is_budget_exhausted_message`];
/// * a BYO provider's insufficient-credits 402, matched by
///   [`crate::core::observability::is_insufficient_credits_message`].
///
/// Both delegate to the crate's single-source classifiers so the phrase sets
/// can't drift from the cron halt / `before_send` nets that share them.
fn is_expected_billing_failure(message: &str) -> bool {
    crate::openhuman::inference::provider::is_budget_exhausted_message(message)
        || crate::core::observability::is_insufficient_credits_message(message)
}

/// Log a failed `context_scout` run at the severity its cause deserves.
///
/// The scout is a **background/best-effort** pass: every caller already
/// degrades to the un-augmented message on failure. When the cause is the user
/// being out of credits (`USER_INSUFFICIENT_CREDITS`) that is a preventable
/// billing state OpenHuman has no lever over, yet `tracing::error!` maps to
/// `EventFilter::Event` in `core::logging::sentry_tracing_layer` — so every
/// tick of every out-of-credits user paged Sentry (TAURI-RUST-HMW: 8314 events
/// / 13 users, #5308). The existing `is_budget_event` `before_send` net cannot
/// catch it: that filter is tag-gated (`failure=non_2xx` + `status=400`) and
/// keys on the event *message*, but this event's message is the static
/// `"context_scout run failed"` with the wire body only in a breadcrumb.
///
/// So demote at the emit site: `warn!` maps to `EventFilter::Breadcrumb`, which
/// keeps the local log line (and the Sentry breadcrumb trail for any *real*
/// error that follows) without raising an issue. Every other cause still
/// `error!`s and keeps paging.
fn log_scout_failure(error_kind: &str, message: &str) {
    if is_expected_billing_failure(message) {
        // Metadata-only — never log the raw provider body (see CLAUDE.md).
        tracing::warn!(
            target: "agent_prepare_context",
            error_kind = %error_kind,
            "[agent_prepare_context] context_scout run skipped — account is out of credits (expected user-state, not reported)"
        );
        return;
    }
    tracing::error!(
        target: "agent_prepare_context",
        error_kind = %error_kind,
        "[agent_prepare_context] context_scout run failed"
    );
}

async fn run_context_scout_with_catalog_and_workspace(
    question: &str,
    focus: Option<&str>,
    tool_catalog: &str,
    parent_workspace_descriptor: Option<WorkspaceDescriptor>,
) -> anyhow::Result<ToolResult> {
    let question = question.trim().to_string();
    let focus = focus.map(|s| s.to_string());

    tracing::info!(
        target: "agent_prepare_context",
        question_chars = question.chars().count(),
        has_focus = focus.as_deref().map(|f| !f.trim().is_empty()).unwrap_or(false),
        "[agent_prepare_context] invoked"
    );

    if question.is_empty() {
        return Ok(ToolResult::error(
            "agent_prepare_context: `question` is required",
        ));
    }

    let registry = match AgentDefinitionRegistry::global() {
        Some(reg) => reg,
        None => {
            return Ok(ToolResult::error(
                "agent_prepare_context: AgentDefinitionRegistry has not been initialised.",
            ));
        }
    };
    let definition = match registry.get(SCOUT_AGENT_ID) {
        Some(def) => def,
        None => {
            return Ok(ToolResult::error(format!(
                "agent_prepare_context: built-in agent `{SCOUT_AGENT_ID}` is not registered.",
            )));
        }
    };

    let catalog_tool_count = tool_catalog.lines().filter(|l| !l.is_empty()).count();
    let scout_prompt =
        AgentPrepareContextTool::build_scout_prompt(&question, focus.as_deref(), tool_catalog);

    tracing::debug!(
        target: "agent_prepare_context",
        catalog_tool_count,
        scout_prompt_chars = scout_prompt.chars().count(),
        "[agent_prepare_context] spawning context_scout (blocking)"
    );

    let task_id = format!("ctx-{}", uuid::Uuid::new_v4());
    let parent_session = current_parent()
        .map(|p| p.session_id.clone())
        .unwrap_or_else(|| "standalone".into());
    let progress_sink = current_parent().and_then(|p| p.on_progress.clone());

    // Surface the scout as a live subagent row in the parent thread. The
    // child's own iterations/tool-calls already stream to this sink from
    // inside run_subagent; we bookend them with spawned/completed so the
    // UI opens and closes the card. Best-effort — a closed sink is fine.
    crate::openhuman::agent::orchestration::subagent_events::publish_subagent_spawned(
        parent_session.clone(),
        definition.id.clone(),
        "typed".to_string(),
        task_id.clone(),
        scout_prompt.chars().count(),
    );
    if let Some(ref tx) = progress_sink {
        let _ = tx
            .send(AgentProgress::SubagentSpawned {
                agent_id: definition.id.clone(),
                task_id: task_id.clone(),
                mode: "typed".to_string(),
                dedicated_thread: false,
                prompt_chars: scout_prompt.chars().count(),
                prompt: scout_prompt.clone(),
                worker_thread_id: None,
                display_name: Some(definition.display_name().to_string()),
            })
            .await;
    }

    let worktree_action_dir = parent_workspace_descriptor
        .as_ref()
        .map(|descriptor| descriptor.root.clone());
    if let Some(descriptor) = parent_workspace_descriptor.as_ref() {
        tracing::debug!(
            target: "agent_prepare_context",
            task_id = %task_id,
            workspace_root = %descriptor.root.display(),
            policy_id = %descriptor.policy_id,
            "[agent_prepare_context] using ToolExecutionContext workspace root"
        );
    }
    let options = SubagentRunOptions {
        task_id: Some(task_id.clone()),
        worktree_action_dir,
        workspace_descriptor: parent_workspace_descriptor,
        ..Default::default()
    };

    match run_subagent(definition, &scout_prompt, options).await {
        Ok(outcome) => match &outcome.status {
            SubagentRunStatus::Completed => {
                // Guard the contract: the scout MUST return exactly one
                // `[context_bundle] … [/context_bundle]` envelope. We tolerate
                // surrounding prose by extracting just the envelope (the harness
                // prepends any non-error result to turn 1 as "Prepared context",
                // so we still inject only the bracketed envelope, never the
                // model's free-form text). Genuinely unusable output — absent,
                // unterminated, or duplicated — is rejected so the caller falls
                // back to the un-augmented message.
                let Some(bundle) = extract_context_bundle(&outcome.output) else {
                    tracing::warn!(
                        target: "agent_prepare_context",
                        task_id = %outcome.task_id,
                        output_chars = outcome.output.chars().count(),
                        "[agent_prepare_context] scout returned a malformed/absent context_bundle — rejecting"
                    );
                    crate::openhuman::agent::orchestration::subagent_events::publish_subagent_completed(
                        parent_session.clone(),
                        outcome.task_id.clone(),
                        outcome.agent_id.clone(),
                        outcome.elapsed.as_millis() as u64,
                        0,
                        outcome.iterations,
                    );
                    if let Some(ref tx) = progress_sink {
                        let _ = tx
                            .send(AgentProgress::SubagentCompleted {
                                agent_id: outcome.agent_id.clone(),
                                task_id: outcome.task_id.clone(),
                                elapsed_ms: outcome.elapsed.as_millis() as u64,
                                iterations: outcome.iterations as u32,
                                output_chars: 0,
                                output: String::new(),
                                worktree_path: None,
                                changed_files: Vec::new(),
                                dirty_status: None,
                            })
                            .await;
                    }
                    return Ok(ToolResult::error(
                        "agent_prepare_context: context_scout did not return a well-formed \
                         [context_bundle] envelope",
                    ));
                };
                // From here on use the extracted `bundle`, not the raw
                // `outcome.output`, so any prose the scout wrapped around the
                // envelope never reaches the parent's context.
                tracing::info!(
                    target: "agent_prepare_context",
                    task_id = %outcome.task_id,
                    elapsed_ms = outcome.elapsed.as_millis() as u64,
                    iterations = outcome.iterations,
                    output_chars = bundle.chars().count(),
                    raw_output_chars = outcome.output.chars().count(),
                    "[agent_prepare_context] context bundle ready"
                );
                crate::openhuman::agent::orchestration::subagent_events::publish_subagent_completed(
                    parent_session.clone(),
                    outcome.task_id.clone(),
                    outcome.agent_id.clone(),
                    outcome.elapsed.as_millis() as u64,
                    bundle.chars().count(),
                    outcome.iterations,
                );
                if let Some(ref tx) = progress_sink {
                    let _ = tx
                        .send(AgentProgress::SubagentCompleted {
                            agent_id: outcome.agent_id.clone(),
                            task_id: outcome.task_id.clone(),
                            elapsed_ms: outcome.elapsed.as_millis() as u64,
                            iterations: outcome.iterations as u32,
                            output_chars: bundle.chars().count(),
                            output: bundle.clone(),
                            worktree_path: None,
                            changed_files: Vec::new(),
                            dirty_status: None,
                        })
                        .await;
                }

                // Bootstrap this thread's goal from the scout's proposal — but
                // ONLY when the thread has none yet. The orchestrator stays
                // authoritative (it sets/replaces via `goal_set`); the
                // context-gathering path just seeds a goal on the first scout of
                // a fresh chat so the harness has something to steer toward.
                // Runs for both entry points (LLM-invoked tool + harness
                // super-context first turn). Best-effort — never fails the call.
                if let (Some(parent), Some(thread_id)) = (current_parent(), current_thread_id()) {
                    if let Some(objective) = AgentPrepareContextTool::parse_proposed_goal(&bundle) {
                        match crate::openhuman::threads::goals::store::set_if_absent(
                            &parent.workspace_dir,
                            &thread_id,
                            &objective,
                            None,
                        )
                        .await
                        {
                            Ok(Some(goal)) => {
                                tracing::info!(
                                    target: "agent_prepare_context",
                                    thread_id = %thread_id,
                                    goal_id = %goal.goal_id,
                                    "[agent_prepare_context] bootstrapped thread goal from scout proposal"
                                );
                                publish_global(DomainEvent::ThreadGoalUpdated {
                                    thread_id: goal.thread_id.clone(),
                                    goal_id: goal.goal_id.clone(),
                                    status: goal.status.as_str().to_string(),
                                });
                            }
                            Ok(None) => {
                                tracing::debug!(
                                    target: "agent_prepare_context",
                                    thread_id = %thread_id,
                                    "[agent_prepare_context] thread already has a goal — scout proposal not applied"
                                );
                            }
                            Err(e) => {
                                tracing::debug!(
                                    target: "agent_prepare_context",
                                    error = %e,
                                    "[agent_prepare_context] failed to persist scout-proposed goal"
                                );
                            }
                        }
                    }
                }

                Ok(ToolResult::success(bundle))
            }
            // The scout has no `ask_user_clarification` tool, so this
            // branch should not fire — handle defensively rather than
            // leaking a confusing checkpoint envelope to the parent.
            SubagentRunStatus::AwaitingUser { question, .. } => {
                tracing::warn!(
                    target: "agent_prepare_context",
                    task_id = %outcome.task_id,
                    "[agent_prepare_context] scout unexpectedly awaited user input"
                );
                // Close the domain-event lifecycle too — a SubagentSpawned
                // was already published, so emit Completed to avoid a
                // dangling spawned state for event-bus consumers.
                crate::openhuman::agent::orchestration::subagent_events::publish_subagent_completed(
                    parent_session.clone(),
                    outcome.task_id.clone(),
                    outcome.agent_id.clone(),
                    outcome.elapsed.as_millis() as u64,
                    0,
                    outcome.iterations,
                );
                if let Some(ref tx) = progress_sink {
                    let _ = tx
                        .send(AgentProgress::SubagentCompleted {
                            agent_id: outcome.agent_id.clone(),
                            task_id: outcome.task_id.clone(),
                            elapsed_ms: outcome.elapsed.as_millis() as u64,
                            iterations: outcome.iterations as u32,
                            output_chars: 0,
                            output: String::new(),
                            worktree_path: None,
                            changed_files: Vec::new(),
                            dirty_status: None,
                        })
                        .await;
                }
                Ok(ToolResult::success(format!(
                    "[context_bundle]\nhas_enough_context: false\n\
                     summary: The context scout could not complete without clarification: {question}\n\
                     recommended_tool_calls:\n[/context_bundle]"
                )))
            }
            SubagentRunStatus::Incomplete { reason } => {
                // The scout stopped short (stuck halt / iteration cap) without a
                // well-formed bundle. Don't inject partial context — return a
                // has_enough_context:false bundle and close the lifecycle.
                tracing::warn!(
                    target: "agent_prepare_context",
                    task_id = %outcome.task_id,
                    reason = %reason,
                    "[agent_prepare_context] scout stopped incomplete — returning empty bundle"
                );
                crate::openhuman::agent::orchestration::subagent_events::publish_subagent_completed(
                    parent_session.clone(),
                    outcome.task_id.clone(),
                    outcome.agent_id.clone(),
                    outcome.elapsed.as_millis() as u64,
                    0,
                    outcome.iterations,
                );
                if let Some(ref tx) = progress_sink {
                    let _ = tx
                        .send(AgentProgress::SubagentCompleted {
                            agent_id: outcome.agent_id.clone(),
                            task_id: outcome.task_id.clone(),
                            elapsed_ms: outcome.elapsed.as_millis() as u64,
                            iterations: outcome.iterations as u32,
                            output_chars: 0,
                            output: String::new(),
                            worktree_path: None,
                            changed_files: Vec::new(),
                            dirty_status: None,
                        })
                        .await;
                }
                Ok(ToolResult::success(format!(
                    "[context_bundle]\nhas_enough_context: false\n\
                     summary: The context scout stopped before finishing ({reason}).\n\
                     recommended_tool_calls:\n[/context_bundle]"
                )))
            }
        },
        Err(err) => {
            let message = err.to_string();
            let error_kind = message
                .split(':')
                .next()
                .map(str::trim)
                .unwrap_or("unknown");
            log_scout_failure(error_kind, &scout_failure_signal(&err));
            crate::openhuman::agent::orchestration::subagent_events::publish_subagent_failed(
                parent_session.clone(),
                task_id.clone(),
                definition.id.clone(),
                message.clone(),
            );
            if let Some(ref tx) = progress_sink {
                let _ = tx
                    .send(AgentProgress::SubagentFailed {
                        agent_id: definition.id.clone(),
                        task_id: task_id.clone(),
                        error: message.clone(),
                    })
                    .await;
            }
            Ok(ToolResult::error(format!(
                "agent_prepare_context failed: {message}"
            )))
        }
    }
}

/// Spawns the `context_scout` sub-agent to collect context and propose a plan.
pub struct AgentPrepareContextTool;

impl Default for AgentPrepareContextTool {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentPrepareContextTool {
    pub fn new() -> Self {
        Self
    }

    /// Render the parent agent's tool catalogue into a compact
    /// `- name: description` list the scout can recommend *back* to the
    /// parent. Excludes this tool itself (recommending another scout pass
    /// would be circular). Returns an empty string when there's no parent
    /// context (e.g. a direct CLI/RPC tool call outside an agent turn) — the
    /// subsequent `run_subagent` call surfaces the no-parent error.
    ///
    /// Restricted to the parent's **visible** tool set (what it actually
    /// advertises and will execute this turn), not the full registry —
    /// otherwise the scout could recommend hidden direct-exec/spawn tools
    /// the parent can't call, which the runtime would reject or which would
    /// bypass specialist routing. Falls back to the full registry only when
    /// the visible set is unknown (empty), to preserve behaviour in contexts
    /// that don't populate it.
    fn render_parent_tool_catalog() -> String {
        let Some(parent) = current_parent() else {
            return String::new();
        };
        let visible = &parent.visible_tool_names;
        let mut out = String::with_capacity(2048);
        for spec in parent.all_tool_specs.iter() {
            if spec.name == "agent_prepare_context" {
                continue;
            }
            if !visible.is_empty() && !visible.contains(&spec.name) {
                continue;
            }
            // One line per tool; trim the description to keep the catalogue
            // from dwarfing the scout's own prompt.
            let desc: String = spec
                .description
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let desc = if desc.chars().count() > 160 {
                let cut = desc
                    .char_indices()
                    .nth(160)
                    .map(|(i, _)| i)
                    .unwrap_or(desc.len());
                format!("{}…", &desc[..cut])
            } else {
                desc
            };
            let _ = writeln!(out, "- {}: {}", spec.name, desc);
        }
        out
    }

    /// Build the scout's task prompt: the request, optional focus, and the
    /// parent tool catalogue the scout draws its recommendations from.
    fn build_scout_prompt(question: &str, focus: Option<&str>, tool_catalog: &str) -> String {
        let mut prompt = String::with_capacity(question.len() + tool_catalog.len() + 512);
        let _ = writeln!(prompt, "[Request]\n{question}\n");
        if let Some(focus) = focus.filter(|f| !f.trim().is_empty()) {
            let _ = writeln!(prompt, "[Focus]\n{}\n", focus.trim());
        }
        if tool_catalog.trim().is_empty() {
            prompt.push_str(
                "[Orchestrator tools]\n(none available — return an empty \
                 recommended_tool_calls list)\n",
            );
        } else {
            let _ = writeln!(
                prompt,
                "[Orchestrator tools]\nThese are the tools the orchestrator can call next. \
                 Every `recommended_tool_calls[].tool` MUST be one of these exact names:\n{tool_catalog}"
            );
        }
        prompt.push_str(
            "\nGather what you need, then emit the single [context_bundle] … \
             [/context_bundle] block as specified. Do not answer the request yourself.",
        );
        prompt
    }

    /// Extract the scout's `proposed_goal:` line from a `[context_bundle]`, if
    /// present and meaningful. Returns `None` for a missing line or an explicit
    /// `none`. The prefix is matched case-insensitively; its byte length is
    /// fixed (no multibyte), so slicing past it is safe.
    fn parse_proposed_goal(bundle: &str) -> Option<String> {
        const PREFIX: &str = "proposed_goal:";
        // Boundary-safe prefix match: `get(..len)` returns None rather than
        // panicking when the line begins with a multibyte char before byte 14.
        let line = bundle.lines().map(str::trim).find(|l| {
            l.get(..PREFIX.len())
                .is_some_and(|p| p.eq_ignore_ascii_case(PREFIX))
        })?;
        let value = line[PREFIX.len()..].trim();
        if value.is_empty() || value.eq_ignore_ascii_case("none") {
            return None;
        }
        Some(value.to_string())
    }
}

#[async_trait]
impl Tool for AgentPrepareContextTool {
    fn name(&self) -> &str {
        "agent_prepare_context"
    }

    fn description(&self) -> &str {
        "Before answering or delegating, scout existing context. Runs a fast \
         read-only context-collector that checks memory, past conversations \
         (transcripts), your goals/profile, installed/registry skills, connected \
         integrations, and the web, then returns whether there's enough context \
         to answer, a compact context summary, an ordered list of recommended \
         next tool calls (parent tools, by exact name, with args), and any \
         skills worth running. Use only when a caller explicitly needs an \
         ad hoc scout pass. If the current prompt says agent context has \
         already been prepared, use the prepared context and do not call this \
         tool again."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["question"],
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The user's request or goal to gather context for. Be specific — the scout has no memory of your conversation."
                },
                "focus": {
                    "type": "string",
                    "description": "Optional hint that narrows what to scout (e.g. a platform, time window, or sub-question)."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // ReadOnly, not Execute: this tool only ever runs the read-only
        // `context_scout` (read_only sandbox, no write/exec tools). Marking it
        // Execute would make `ToolPolicyEngine` strip it from any
        // provider-visible set on a `ReadOnly`-capped channel, which would hide
        // the scout from callers that still expose it explicitly.
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: serde_json::Value,
        _options: ToolCallOptions,
        tool_context: Option<&ToolExecutionContext>,
    ) -> anyhow::Result<ToolResult> {
        let prepared_sources = current_agent_context_prepared_sources();
        if !prepared_sources.is_empty() {
            tracing::info!(
                target: "agent_prepare_context",
                sources = ?prepared_sources,
                "[agent_prepare_context] skipped because agent context is already prepared for this turn"
            );
            return Ok(ToolResult::success(already_prepared_context_bundle(
                &prepared_sources,
            )));
        }

        let question = args.get("question").and_then(|v| v.as_str()).unwrap_or("");
        let focus = args.get("focus").and_then(|v| v.as_str());
        let tool_catalog = AgentPrepareContextTool::render_parent_tool_catalog();
        run_context_scout_with_catalog_and_workspace(
            question,
            focus,
            &tool_catalog,
            tool_context.and_then(|ctx| ctx.workspace.clone()),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_requires_question_and_makes_focus_optional() {
        let tool = AgentPrepareContextTool::new();
        let schema = tool.parameters_schema();
        let required = schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("schema has required array");
        assert!(required.iter().any(|v| v.as_str() == Some("question")));
        assert!(
            required.iter().all(|v| v.as_str() != Some("focus")),
            "focus must be optional"
        );
        let props = schema.get("properties").expect("schema has properties");
        assert!(props.get("question").is_some());
        assert!(props.get("focus").is_some());
    }

    #[test]
    fn description_skips_when_context_is_already_prepared() {
        let tool = AgentPrepareContextTool::new();
        let description = tool.description();

        assert!(description.contains("agent context has already been prepared"));
        assert!(description.contains("do not call this tool again"));
    }

    #[test]
    fn build_scout_prompt_includes_request_focus_and_catalog() {
        let prompt = AgentPrepareContextTool::build_scout_prompt(
            "summarise my unread gmail",
            Some("last 24h"),
            "- delegate_to_integrations_agent: route to a connected integration\n",
        );
        assert!(prompt.contains("[Request]"));
        assert!(prompt.contains("summarise my unread gmail"));
        assert!(prompt.contains("[Focus]"));
        assert!(prompt.contains("last 24h"));
        assert!(prompt.contains("[Orchestrator tools]"));
        assert!(prompt.contains("delegate_to_integrations_agent"));
        assert!(prompt.contains("[context_bundle]"));
    }

    #[test]
    fn build_scout_prompt_handles_empty_catalog() {
        let prompt = AgentPrepareContextTool::build_scout_prompt("do a thing", None, "");
        assert!(prompt.contains("(none available"));
        assert!(!prompt.contains("[Focus]"));
    }

    #[test]
    fn parse_proposed_goal_extracts_objective_or_none() {
        let bundle = "[context_bundle]\nhas_enough_context: true\n\
                      proposed_goal: Ship the desktop release to all platforms\n\
                      summary: ...\n[/context_bundle]";
        assert_eq!(
            AgentPrepareContextTool::parse_proposed_goal(bundle).as_deref(),
            Some("Ship the desktop release to all platforms")
        );

        // Explicit `none` → no goal.
        let none_bundle = "[context_bundle]\nproposed_goal: none\nsummary: x\n[/context_bundle]";
        assert!(AgentPrepareContextTool::parse_proposed_goal(none_bundle).is_none());

        // Missing line → no goal.
        let no_line = "[context_bundle]\nhas_enough_context: true\n[/context_bundle]";
        assert!(AgentPrepareContextTool::parse_proposed_goal(no_line).is_none());

        // Case-insensitive prefix.
        let cased = "Proposed_Goal:  Land the migration  ";
        assert_eq!(
            AgentPrepareContextTool::parse_proposed_goal(cased).as_deref(),
            Some("Land the migration")
        );

        // Lines starting with a multibyte char must not panic the byte-prefix
        // match (regression for the `l[..14]` non-boundary slice).
        let multibyte = "[context_bundle]\n日本語の要約 summary line\nproposed_goal: 目標を達成する\n[/context_bundle]";
        assert_eq!(
            AgentPrepareContextTool::parse_proposed_goal(multibyte).as_deref(),
            Some("目標を達成する")
        );
    }

    #[tokio::test]
    async fn missing_question_returns_error() {
        let tool = AgentPrepareContextTool::new();
        let result = tool.execute(json!({})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("question"));
    }

    #[tokio::test]
    async fn execute_short_circuits_when_context_already_prepared() {
        let tool = AgentPrepareContextTool::new();
        let result = crate::openhuman::agent::harness::with_agent_context_prepared_sources(
            vec![AgentContextPreparedSource {
                source: "super context preparation".to_string(),
                has_enough_context: Some(false),
            }],
            tool.execute(json!({"question": "prepare context again"})),
        )
        .await
        .unwrap();

        assert!(!result.is_error, "{}", result.output());
        assert!(result.output().contains("[context_bundle]"));
        assert!(result.output().contains("has_enough_context: false"));
        assert!(result.output().contains("already been prepared once"));
        assert!(result.output().contains("super context preparation"));
        assert!(result
            .output()
            .contains("does not assert that enough context is available"));
        assert!(result.output().contains("[/context_bundle]"));
    }

    #[tokio::test]
    async fn execute_preserves_prior_prepared_context_sufficiency_when_true() {
        let tool = AgentPrepareContextTool::new();
        let result = crate::openhuman::agent::harness::with_agent_context_prepared_sources(
            vec![AgentContextPreparedSource {
                source: "super context preparation".to_string(),
                has_enough_context: Some(true),
            }],
            tool.execute(json!({"question": "prepare context again"})),
        )
        .await
        .unwrap();

        assert!(!result.is_error, "{}", result.output());
        assert!(result.output().contains("has_enough_context: true"));
        assert!(result.output().contains("reported enough context"));
    }

    #[test]
    fn extracts_a_single_well_formed_bundle() {
        let out = "[context_bundle]\nhas_enough_context: true\nsummary: ok\n[/context_bundle]";
        assert_eq!(extract_context_bundle(out).as_deref(), Some(out));
    }

    #[test]
    fn rejects_free_form_prose_without_a_bundle() {
        assert_eq!(
            extract_context_bundle("Sure! Here's what I found about your request..."),
            None
        );
    }

    #[test]
    fn rejects_unterminated_or_reversed_envelope() {
        // Open tag with no close.
        assert_eq!(
            extract_context_bundle("[context_bundle]\nsummary: ..."),
            None
        );
        // Close before open — out of order.
        assert_eq!(
            extract_context_bundle("[/context_bundle] stray [context_bundle]"),
            None
        );
    }

    #[test]
    fn rejects_duplicated_envelope() {
        // Two envelopes — we can't tell which is authoritative, so reject.
        assert_eq!(
            extract_context_bundle(
                "[context_bundle]a[/context_bundle][context_bundle]b[/context_bundle]"
            ),
            None
        );
    }

    #[test]
    fn extracts_envelope_from_surrounding_prose() {
        // Regression for the "scout runs, bundle missing" bug: a fast chat-tier
        // scout wraps the envelope in a preamble and/or a closing line. We must
        // extract just the envelope, not drop it and not inject the prose.
        let leading = "Sure, here's what I found:\n[context_bundle]\nsummary: x\n[/context_bundle]";
        assert_eq!(
            extract_context_bundle(leading).as_deref(),
            Some("[context_bundle]\nsummary: x\n[/context_bundle]")
        );
        let trailing = "[context_bundle]\nsummary: x\n[/context_bundle]\nHope that helps!";
        assert_eq!(
            extract_context_bundle(trailing).as_deref(),
            Some("[context_bundle]\nsummary: x\n[/context_bundle]")
        );
        let both = "Here you go:\n[context_bundle]\nsummary: x\n[/context_bundle]\n\nLet me know!";
        assert_eq!(
            extract_context_bundle(both).as_deref(),
            Some("[context_bundle]\nsummary: x\n[/context_bundle]")
        );
    }

    #[test]
    fn extracts_envelope_with_surrounding_whitespace() {
        // Leading/trailing whitespace is trimmed, not treated as prose.
        assert_eq!(
            extract_context_bundle("\n  [context_bundle]\nsummary: x\n[/context_bundle]\n  ")
                .as_deref(),
            Some("[context_bundle]\nsummary: x\n[/context_bundle]")
        );
    }

    // ── Credits-exhausted scout failures stay off Sentry (#5308) ────────

    /// Verbatim wire body from Sentry TAURI-RUST-HMW's breadcrumb — the
    /// managed backend's budget-exhausted 400. Pinning the exact string makes a
    /// backend phrasing drift fail CI rather than silently re-flood Sentry.
    const CREDITS_400_BODY: &str = "OpenHuman returned HTTP 400: \
        {\"success\":false,\"error\":\"Insufficient budget\",\
        \"errorCode\":\"USER_INSUFFICIENT_CREDITS\"}";

    #[test]
    fn classifies_managed_credits_400_and_byo_402_as_expected_billing() {
        assert!(
            is_expected_billing_failure(CREDITS_400_BODY),
            "the managed USER_INSUFFICIENT_CREDITS 400 is user-state, not a defect"
        );
        // The BYO sibling: a provider 402 whose body names the credit shortfall.
        assert!(is_expected_billing_failure(
            "openrouter API error (402 Payment Required): {\"error\":\"Insufficient credits\"}"
        ));
    }

    #[test]
    fn real_scout_failures_are_not_classified_as_billing() {
        for message in [
            "provider call failed: connection reset by peer",
            "agent definition 'context_scout' not found in registry",
            "sub-agent exceeded maximum iterations (8)",
            "OpenHuman returned HTTP 400: {\"error\":\"model not found\"}",
            // A healthy-balance readout must not be swallowed.
            "You have 100 remaining credits this month",
            "",
        ] {
            assert!(
                !is_expected_billing_failure(message),
                "{message:?} must keep paging Sentry"
            );
        }
    }

    #[test]
    fn scout_failure_signal_flattens_the_provider_cause_chain() {
        // The wire body arrives as a *cause* under the runner's own context, so
        // `to_string()` alone loses it — the classifier would then miss the very
        // failure the demotion exists for.
        let inner = anyhow::Error::msg(CREDITS_400_BODY).context("chat completion failed");
        let err = SubagentRunError::Provider(inner);

        assert!(
            !err.to_string().contains("USER_INSUFFICIENT_CREDITS"),
            "precondition: the flat Display drops the cause carrying the wire body"
        );
        let signal = scout_failure_signal(&err);
        assert!(signal.contains("USER_INSUFFICIENT_CREDITS"));
        assert!(is_expected_billing_failure(&signal));

        // Non-`Provider` variants are already flat thiserror messages.
        assert_eq!(
            scout_failure_signal(&SubagentRunError::MaxIterationsExceeded(8)),
            "sub-agent exceeded maximum iterations (8)"
        );
    }

    /// The regression this issue is about: a background `context_scout` run that
    /// fails on exhausted credits must NOT become a Sentry error event, while
    /// every other failure still does.
    ///
    /// Asserted end-to-end through the real `sentry-tracing` bridge (mirroring
    /// `core::logging::sentry_tracing_layer`'s level mapping) rather than by
    /// inspecting the log level, because the level→event mapping *is* the
    /// mechanism: `ERROR` → `EventFilter::Event`, `WARN` → `Breadcrumb`.
    #[cfg(feature = "crash-reporting")]
    #[test]
    fn credits_exhausted_scout_failure_does_not_reach_sentry() {
        use sentry::test::TestTransport;
        use std::sync::Arc;
        use tracing::Level;
        use tracing_subscriber::layer::SubscriberExt;

        let transport = TestTransport::new();
        let options = sentry::ClientOptions {
            dsn: Some("https://public@sentry.invalid/1".parse().unwrap()),
            transport: Some(Arc::new(transport.clone())),
            ..Default::default()
        };
        let hub = Arc::new(sentry::Hub::new(
            Some(Arc::new(options.into())),
            Arc::new(Default::default()),
        ));
        let _hub_guard = sentry::HubSwitchGuard::new(hub);

        let subscriber = tracing_subscriber::registry().with(
            sentry::integrations::tracing::layer().event_filter(|metadata| {
                match *metadata.level() {
                    Level::ERROR => sentry::integrations::tracing::EventFilter::Event,
                    Level::WARN | Level::INFO => {
                        sentry::integrations::tracing::EventFilter::Breadcrumb
                    }
                    _ => sentry::integrations::tracing::EventFilter::Ignore,
                }
            }),
        );
        let _subscriber_guard = tracing::subscriber::set_default(subscriber);

        log_scout_failure("provider call failed", CREDITS_400_BODY);
        assert!(
            transport.fetch_and_clear_events().is_empty(),
            "an out-of-credits background scout must not page Sentry (TAURI-RUST-HMW)"
        );

        log_scout_failure("provider call failed", "connection reset by peer");
        let events = transport.fetch_and_clear_events();
        assert_eq!(
            events.len(),
            1,
            "a genuine scout failure must still reach Sentry"
        );
        assert_eq!(events[0].level, sentry::Level::Error);
    }
}
