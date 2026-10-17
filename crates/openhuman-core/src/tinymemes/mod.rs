//! TinyMemes host adapter: remixes the orchestrator's final reply with
//! region slang and memes when the chat can take it.
//!
//! The engine (Jev reading, rating, rewrite, slang and meme indexes) runs in
//! the `tinymemes` module, reached over its `tinymemes-bus` contract. This
//! domain keeps what is the host's:
//!
//! - the `OPENHUMAN_TINYMEMES` flag and the A/B split ([`arm`]);
//! - the thread as the user saw it, each message named by its id
//!   ([`history`]); the module keeps the ring of remixed ids itself and tags
//!   earlier remixed replies from it;
//! - the callbacks the module asks for while it works: OpenHuman inference,
//!   managed Jev, web search and the learned-state documents ([`callbacks`],
//!   [`docs`], served over the bus by [`service`]). Each carries the remix's
//!   correlation id, so remixes for different workspaces run side by side.
//!
//! The module also owns the `TINYMEMES_*` environment overrides (OpenRouter key
//! and model, Jev route), the LLM fallback when this host has no Jev, and the
//! slang and meme research it runs in the background after delivery.
//!
//! Behaviour:
//!
//! - Runs after the turn, on `full_response` only. Sub-agent output never
//!   reaches it.
//! - The agent's own transcript keeps the original wording, so the
//!   orchestrator never sees its replies in slang.
//! - Fails open: any error, refusal or timeout delivers the original reply.
//!
//! Every turn in an experiment logs one grep-friendly `[tinymemes] turn` line
//! (arm, turn time, remix time, rating, outcome) for the A/B comparison. No
//! user content is logged.

mod arm;
mod callbacks;
mod docs;
mod history;
mod inference;
mod jev;
mod search;
pub(crate) mod service;

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use async_trait::async_trait;
use std::time::{Duration, Instant};
use tinymemes_bus::{RemixOptions, RemixRequest, RemixResponse, SkipReason};

pub(crate) use arm::Arm;
pub(crate) use arm::FLAG_ENV;

/// Whether an `OPENHUMAN_TINYMEMES` value turns remixing on in any form.
pub(crate) fn flag_enabled(value: &str) -> bool {
    arm::mode_from(Some(value)) != arm::Mode::Off
}

const TIMEOUT_ENV: &str = "OPENHUMAN_TINYMEMES_TIMEOUT_MS";
const COOLDOWN_ENV: &str = "OPENHUMAN_TINYMEMES_MEME_COOLDOWN";

/// Whether this thread's answer text must be held back from streaming because
/// its final reply will be remixed (treatment arm). The UI then shows only the
/// remixed reply instead of the original streaming in and being replaced.
pub(crate) fn holds_text_stream(config: &Config, thread_id: &str) -> bool {
    arm::arm_for(config, arm::mode(), thread_id) == Arm::Treatment
}

/// Something that can remix one reply: the module in production, a script in
/// tests.
#[async_trait]
pub(crate) trait Remixer: Send + Sync {
    async fn remix(&self, request: RemixRequest) -> Result<RemixResponse, String>;
}

/// The loadable `tinymemes` module, with this workspace's callbacks served.
struct ModuleRemixer<'a> {
    ctx: &'a DispatchCtx,
    config: &'a Config,
}

#[async_trait]
impl Remixer for ModuleRemixer<'_> {
    async fn remix(&self, request: RemixRequest) -> Result<RemixResponse, String> {
        service::remix(self.ctx, self.config, request).await
    }
}

/// Remix a finished task's reply in place, when it is a real answer (not
/// empty, not the `placeholder` sent when the inference budget ran out) and
/// the thread is in the treatment arm. Fails open: the reply is left
/// untouched on any error.
pub(crate) async fn remix_task_reply(
    ctx: &DispatchCtx,
    config: &Config,
    thread_id: &str,
    request_id: &str,
    user_message: &str,
    full_response: &mut String,
    placeholder: &str,
    turn_elapsed: Duration,
) {
    if !history::should_remix(full_response, placeholder) {
        return;
    }
    if let Some(remixed) = remix_final_reply(
        arm::arm_for(config, arm::mode(), thread_id),
        ctx,
        config,
        thread_id,
        request_id,
        user_message,
        full_response,
        turn_elapsed,
    )
    .await
    {
        *full_response = remixed;
    }
}

/// Remix a finished turn's reply. Returns the text to deliver instead, or
/// `None` to deliver the original. `turn_elapsed` is the agent turn's own
/// duration, logged for both arms so throughput can be compared.
async fn remix_final_reply(
    arm: Arm,
    ctx: &DispatchCtx,
    config: &Config,
    thread_id: &str,
    request_id: &str,
    user_message: &str,
    reply: &str,
    turn_elapsed: Duration,
) -> Option<String> {
    let turn = TurnInfo {
        request_id,
        bucket: arm::bucket(thread_id),
        turn_ms: turn_elapsed.as_millis(),
    };
    if !gate(arm, &turn) {
        return None;
    }
    let started = Instant::now();
    // Fail open: without the thread, the reading would judge the reply out of
    // context, so the original goes out instead.
    let Some(messages) = load(ctx, config, thread_id).await else {
        log_outcome(&turn, started, "history_unavailable", None);
        return None;
    };
    let remixer = ModuleRemixer { ctx, config };
    remix_with(
        &remixer,
        options(ctx, config),
        &messages,
        &turn,
        started,
        user_message,
        reply,
        budget(config),
    )
    .await
}

/// Per-turn identifiers for the A/B log line.
#[derive(Clone, Copy)]
struct TurnInfo<'a> {
    request_id: &'a str,
    bucket: u8,
    turn_ms: u128,
}

/// Whether the arm runs the remix. The control arm logs its turn for the
/// throughput comparison; a disabled flag does nothing at all.
fn gate(arm: Arm, turn: &TurnInfo<'_>) -> bool {
    match arm {
        Arm::Disabled => false,
        Arm::Control => {
            log::info!(
                "[tinymemes] turn arm={} bucket={} request_id={} turn_ms={} remix_ms=0 \
                 outcome=not_remixed",
                arm.as_str(),
                turn.bucket,
                turn.request_id,
                turn.turn_ms
            );
            false
        }
        Arm::Treatment => true,
    }
}

/// The thread as stored. Blocking file work, so it runs off the async
/// runtime. `None` when the thread cannot be read.
async fn load(
    ctx: &DispatchCtx,
    config: &Config,
    thread_id: &str,
) -> Option<Vec<crate::threads::store::ConversationMessage>> {
    let config = config.clone();
    let thread_id = thread_id.to_owned();
    let ctx = ctx.clone();
    ctx.clone()
        .spawn_blocking(move || {
            crate::threads::store::get_messages(&ctx, config.workspace_dir.clone(), &thread_id)
                .map_err(|e| log::warn!("[tinymemes] thread history unavailable: {e}"))
                .ok()
        })
        .await
        .ok()
        .flatten()
}

/// What the host decides for the module: the meme cooldown
/// (`OPENHUMAN_TINYMEMES_MEME_COOLDOWN` over `[memes] meme_cooldown`) and
/// whether web search can serve research at all.
fn options(ctx: &DispatchCtx, config: &Config) -> RemixOptions {
    options_with(
        std::env::var(COOLDOWN_ENV).ok().as_deref(),
        config.memes.meme_cooldown,
        search::OpenHumanSearch::available(ctx, config).is_some(),
    )
}

fn options_with(env: Option<&str>, configured: u32, research: bool) -> RemixOptions {
    RemixOptions {
        meme_cooldown_turns: Some(
            env.and_then(|v| v.trim().parse::<u32>().ok())
                .unwrap_or(configured),
        ),
        research,
    }
}

/// The remix time budget: `OPENHUMAN_TINYMEMES_TIMEOUT_MS` when set, else
/// `[memes] timeout_ms`.
fn budget(config: &Config) -> Duration {
    budget_with(
        std::env::var(TIMEOUT_ENV).ok().as_deref(),
        config.memes.timeout_ms,
    )
}

fn budget_with(env: Option<&str>, configured_ms: u64) -> Duration {
    Duration::from_millis(
        env.and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(configured_ms),
    )
}

/// Read, rate, and remix `reply` against the thread, within `budget`.
#[allow(clippy::too_many_arguments)]
async fn remix_with(
    remixer: &dyn Remixer,
    options: RemixOptions,
    messages: &[crate::threads::store::ConversationMessage],
    turn: &TurnInfo<'_>,
    started: Instant,
    user_message: &str,
    reply: &str,
    budget: Duration,
) -> Option<String> {
    let request = RemixRequest {
        conversation: history::history_turns(messages, user_message),
        reply: reply.to_owned(),
        region: None,
        enabled: true,
        timeout_ms: u64::try_from(budget.as_millis()).unwrap_or(u64::MAX),
        deliver: false,
        // Set by the service, which owns the registry of live remixes.
        correlation: String::new(),
        // The module remembers this id when the reply goes out remixed.
        reply_id: Some(crate::threads::store::run_reply_message_id(turn.request_id)),
        options,
    };
    // The module enforces `timeout_ms` itself; this outer bound also covers
    // module load, callback serving and the bus hop.
    let response = match tokio::time::timeout(budget, remixer.remix(request)).await {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            log::warn!(
                "[tinymemes] remix unavailable request_id={}: {error}",
                turn.request_id
            );
            log_outcome(turn, started, "error", None);
            return None;
        }
        Err(_) => {
            log_outcome(turn, started, "timeout", None);
            return None;
        }
    };

    // Whether the delivered reply actually changed.
    let changed = response.remixed && response.reply.trim() != reply.trim();
    let result = if changed {
        "remixed"
    } else {
        match response.skipped {
            Some(SkipReason::Timeout) => "timeout",
            Some(SkipReason::Failed) => "error",
            _ => "unchanged",
        }
    };
    log_outcome(turn, started, result, Some(&response));
    changed.then_some(response.reply)
}

fn log_outcome(
    turn: &TurnInfo<'_>,
    started: Instant,
    result: &str,
    response: Option<&RemixResponse>,
) {
    log::info!(
        "{}",
        turn_line(turn, started.elapsed().as_millis(), result, response)
    );
}

/// The treatment arm's per-turn log line. No user content goes in it.
fn turn_line(
    turn: &TurnInfo<'_>,
    remix_ms: u128,
    result: &str,
    response: Option<&RemixResponse>,
) -> String {
    let TurnInfo {
        request_id,
        bucket,
        turn_ms,
    } = *turn;
    format!(
        "[tinymemes] turn arm=treatment bucket={bucket} request_id={request_id} turn_ms={turn_ms} \
         remix_ms={remix_ms} outcome={result} score={} skipped={} wants_more_memes={}",
        response.and_then(|r| r.score).map_or(-1, i32::from),
        response
            .and_then(|r| r.skipped)
            .map_or("none", |s| match s {
                SkipReason::Disabled => "disabled",
                SkipReason::BelowThreshold => "below_threshold",
                SkipReason::Timeout => "timeout",
                SkipReason::Failed => "failed",
            }),
        response.is_some_and(|r| r.wants_more_memes),
    )
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "e2e_tests.rs"]
mod e2e_tests;
