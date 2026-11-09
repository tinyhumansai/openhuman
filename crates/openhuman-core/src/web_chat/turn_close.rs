//! The reply that closes a turn a web-turn guard stopped.
//!
//! When the idle guard or the wall-clock backstop drops a turn, the turn's own
//! loop cannot write its close: it is stuck, or was never reached. Instead of
//! a bare `turn_timeout` error, the guard closes the turn with a reply that
//! says what the turn got done and where it got stuck. The evidence is what
//! the turn recorded while it ran: the turn-state snapshot the progress
//! bridge mirrors (tool timeline and narration), plus the [`StuckPoint`] the
//! activity record saw.
//!
//! One tools-free `summarization` call writes the reply, bounded by
//! [`CLOSE_TIMEOUT`]. The provider may be what stalled, so when that call
//! fails, times out or returns nothing, a plain reply built from the same
//! evidence is used instead. Either way the turn ends in an ordinary
//! delivered reply that is saved in the thread. A turn that stopped before
//! resolving its own config has no workspace to write to and nothing to
//! summarize, so it ends in the guard's `turn_timeout` error instead.

use std::time::Duration;

use tinyagents_session::turn_state::types::TranscriptItem;
use tinyagents_session::turn_state::{ToolTimelineStatus, TurnState};
use tinyinference_llm::message::Message;
use tinyinference_llm::model::ModelRequest;

use crate::inference::provider;

use super::turn_activity::StuckPoint;
use super::types::WebChatTaskResult;

/// Upper bound on the closing model call.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(45);

/// Most tool rows listed in the evidence (the latest ones are kept).
const MAX_STEPS: usize = 40;

/// Most characters of a tool row's detail carried in the evidence.
const MAX_DETAIL_CHARS: usize = 160;

/// Most characters of the agent's latest narration carried in the evidence.
const MAX_NARRATION_CHARS: usize = 1_500;

const LOG_PREFIX: &str = "[web-channel:turn-close]";

const CLOSE_SYSTEM_PROMPT: &str = "You write the closing message of an AI assistant's turn \
    that was stopped before it finished. Write to the user, in the assistant's voice, in \
    plain language. Use only the evidence given; do not invent results. Keep it short, in \
    three parts: what got done, where it got stuck and why the turn was stopped, and what \
    the user can say to continue. Do not apologize at length.";

/// Why a guard stopped the turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StopCause {
    /// No progress, and nothing in flight, for this long.
    Idle(Duration),
    /// The fixed wall-clock ceiling, which the harness wind-down did not beat.
    Ceiling(Duration),
}

impl StopCause {
    fn describe(self) -> String {
        match self {
            Self::Idle(idle) => format!("nothing happened for {}", minutes(idle)),
            Self::Ceiling(limit) => format!("it hit its time limit of {}", minutes(limit)),
        }
    }
}

fn minutes(d: Duration) -> String {
    let (n, unit) = match d.as_secs() {
        secs @ 0..=59 => (secs, "second"),
        secs => (secs / 60, "minute"),
    };
    if n == 1 {
        format!("1 {unit}")
    } else {
        format!("{n} {unit}s")
    }
}

/// A turn a guard stopped, and why.
pub(crate) struct StoppedTurn<'a> {
    pub(crate) thread_id: &'a str,
    pub(crate) request_id: &'a str,
    pub(crate) cause: StopCause,
    pub(crate) stuck: StuckPoint,
    /// The config the turn resolved, when it got that far. Its workspace is
    /// where the turn's snapshot lives and where the reply is saved, even if
    /// the active profile changed since.
    pub(crate) config: Option<std::sync::Arc<crate::config::Config>>,
    /// The turn's reply locale, so the close is written in its language.
    pub(crate) locale: Option<String>,
    /// The context the stopped turn ran under, for the closing model call.
    pub(crate) ctx: crate::core::runtime::DispatchCtx,
}

impl StoppedTurn<'_> {
    /// Whether the agent never received the user's request: the turn stopped
    /// before its run began, so there is nothing to continue from.
    fn request_unseen(&self) -> bool {
        self.stuck == StuckPoint::BeforeRun
    }
}

/// What the turn recorded while it ran, as evidence lines.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Evidence {
    pub(crate) steps: Vec<String>,
    pub(crate) narration: Option<String>,
}

fn truncate(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}…")
}

impl Evidence {
    pub(crate) fn from_snapshot(state: Option<&TurnState>) -> Self {
        let Some(state) = state else {
            return Self::default();
        };
        let skip = state.tool_timeline.len().saturating_sub(MAX_STEPS);
        let steps = state
            .tool_timeline
            .iter()
            .skip(skip)
            .map(|entry| {
                let name = entry.display_name.as_deref().unwrap_or(&entry.name);
                let detail = entry
                    .detail
                    .as_deref()
                    .filter(|d| !d.trim().is_empty())
                    .map(|d| format!(" ({})", truncate(d, MAX_DETAIL_CHARS)))
                    .unwrap_or_default();
                match entry.status {
                    ToolTimelineStatus::Success => format!("done: {name}{detail}"),
                    ToolTimelineStatus::Running => format!("still running: {name}{detail}"),
                    ToolTimelineStatus::Error => {
                        let cause = entry
                            .failure
                            .as_ref()
                            .map(|f| format!(" — {}", f.cause_plain))
                            .unwrap_or_default();
                        format!("failed: {name}{detail}{cause}")
                    }
                }
            })
            .collect();
        let narration = state
            .transcript
            .iter()
            .rev()
            .find_map(|item| match item {
                TranscriptItem::Narration { text, .. } if !text.trim().is_empty() => {
                    Some(text.as_str())
                }
                _ => None,
            })
            .or_else(|| Some(state.streaming_text.as_str()).filter(|t| !t.trim().is_empty()))
            .map(|text| truncate(text, MAX_NARRATION_CHARS));
        Self { steps, narration }
    }

    fn render(&self) -> String {
        let mut out = String::new();
        if self.steps.is_empty() {
            out.push_str("Steps taken: none recorded.\n");
        } else {
            out.push_str("Steps taken, oldest first:\n");
            for step in &self.steps {
                out.push_str("- ");
                out.push_str(step);
                out.push('\n');
            }
        }
        if let Some(narration) = &self.narration {
            out.push_str("\nThe assistant's last message before it stopped:\n");
            out.push_str(narration);
            out.push('\n');
        }
        out
    }
}

/// The plain reply used when the closing model call is unavailable.
pub(crate) fn plain_reply(turn: &StoppedTurn<'_>, evidence: &Evidence) -> String {
    let mut out = format!(
        "I stopped this turn because {}. It got stuck {}.\n\n",
        turn.cause.describe(),
        turn.stuck.describe()
    );
    if evidence.steps.is_empty() {
        out.push_str("No steps had been recorded yet.\n");
    } else {
        out.push_str("What happened before it stopped:\n");
        for step in &evidence.steps {
            out.push_str("- ");
            out.push_str(step);
            out.push('\n');
        }
    }
    if turn.request_unseen() {
        out.push_str(
            "\nYour message didn't reach me before the turn stopped. Please send it again.",
        );
    } else {
        out.push_str("\nSay \"continue\" and I'll pick up from here.");
    }
    out
}

fn close_request(turn: &StoppedTurn<'_>, evidence: &Evidence) -> ModelRequest {
    let next_step = if turn.request_unseen() {
        "The assistant never received the user's request, so it cannot continue it. Ask the \
         user to send the request again; do not suggest saying \"continue\"."
    } else {
        "The user can say \"continue\" to pick up from here."
    };
    let user_prompt = format!(
        "The turn was stopped because {}.\nWhere it got stuck: {}.\n{next_step}\n\n{}",
        turn.cause.describe(),
        turn.stuck.describe(),
        evidence.render()
    );
    let mut system = CLOSE_SYSTEM_PROMPT.to_string();
    if let Some(directive) = turn
        .locale
        .as_deref()
        .and_then(super::session::locale_reply_directive)
    {
        system.push_str("\n\n");
        system.push_str(&directive);
    }
    crate::inference::registry::provider_request_without_reasoning(
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        ModelRequest::new(vec![Message::system(system), Message::user(user_prompt)])
            .with_temperature(0.2),
    )
}

async fn model_reply(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &crate::config::Config,
    turn: &StoppedTurn<'_>,
    evidence: &Evidence,
) -> Option<String> {
    // Tests run offline: the closing model call is skipped and the plain
    // reply is used.
    if cfg!(test) {
        return None;
    }
    let request_id = turn.request_id;
    let (chat_model, model) =
        match provider::create_chat_model_with_model_id_in(ctx, "summarization", config, 0.2).await
        {
            Ok(resolved) => resolved,
            Err(error) => {
                log::warn!("{LOG_PREFIX} provider init failed request_id={request_id}: {error}");
                return None;
            }
        };
    let call = chat_model.invoke(&(), close_request(turn, evidence));
    match tokio::time::timeout(CLOSE_TIMEOUT, call).await {
        Ok(Ok(response)) => Some(response.text()).filter(|text| !text.trim().is_empty()),
        Ok(Err(error)) => {
            log::warn!(
                "{LOG_PREFIX} close call failed request_id={request_id} model={model}: {error}"
            );
            None
        }
        Err(_) => {
            log::warn!(
                "{LOG_PREFIX} close call timed out after {CLOSE_TIMEOUT:?} \
                 request_id={request_id} model={model}"
            );
            None
        }
    }
}

/// Read the turn's snapshot off the async worker: it is a synchronous store
/// read, and a stalled store must not pin the runtime thread. The scoped spawn
/// carries the caller's tenant, which selects the store.
async fn read_snapshot(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace: std::path::PathBuf,
    thread_id: &str,
    request_id: &str,
) -> Option<TurnState> {
    let (thread_id, request_id) = (thread_id.to_string(), request_id.to_string());
    let read = ctx
        .spawn_blocking(move || {
            crate::storage::sessions::turn_states_for(&workspace).get_turn(&thread_id, &request_id)
        })
        .await;
    match read {
        Ok(Ok(snapshot)) => snapshot,
        Ok(Err(error)) => {
            log::warn!("{LOG_PREFIX} turn snapshot unreadable: {error}");
            None
        }
        Err(error) => {
            log::warn!("{LOG_PREFIX} turn snapshot read did not finish: {error}");
            None
        }
    }
}

/// The reply that closes `turn`, or `None` when the turn stopped before it
/// resolved its own config (the caller then reports the stop as an error).
///
/// The reply is written to and read from the turn's own workspace only. A turn
/// that never resolved its config has no workspace of its own, and loading the
/// active profile instead could file the reply under a different user after a
/// credential switch. Such a turn also has nothing to summarize: it stopped
/// before the agent saw the request. The caller bounds the whole call.
pub(crate) async fn close_reply(turn: StoppedTurn<'_>) -> Option<WebChatTaskResult> {
    let Some(config) = turn.config.clone() else {
        log::info!(
            "{LOG_PREFIX} turn stopped before resolving its config; reporting the stop \
             request_id={}",
            turn.request_id
        );
        return None;
    };
    let snapshot = read_snapshot(
        &turn.ctx,
        config.workspace_dir.clone(),
        turn.thread_id,
        turn.request_id,
    )
    .await;
    let evidence = Evidence::from_snapshot(snapshot.as_ref());
    let reply = match model_reply(&turn.ctx, &config, &turn, &evidence).await {
        Some(reply) => reply,
        None => plain_reply(&turn, &evidence),
    };
    log::info!(
        "{LOG_PREFIX} closed stopped turn request_id={} steps={} stuck={:?}",
        turn.request_id,
        evidence.steps.len(),
        turn.stuck
    );
    Some(WebChatTaskResult {
        full_response: reply,
        citations: Vec::new(),
        usage: None,
        workspace_dir: config.workspace_dir.clone(),
        timing: None,
    })
}

#[cfg(test)]
#[path = "turn_close_tests.rs"]
mod tests;
