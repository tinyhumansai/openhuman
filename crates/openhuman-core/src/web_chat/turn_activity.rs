//! Inactivity tracking for one web-chat turn.
//!
//! The web-turn guard (`ops::turn_guards`) used to drop a turn a fixed 900s
//! after it started, whether or not the turn was still working. A turn that
//! streamed a model reply or a tool result every few seconds died exactly as a
//! wedged one did. The guard exists to catch the wedged turn, so it now asks a
//! narrower question: has the turn been **silent with nothing in flight** for
//! the idle window?
//!
//! * Every [`AgentProgress`] event the turn emits restarts the idle clock.
//! * While a parent-scope model call (`IterationStarted` →
//!   `ModelCallCompleted`) or a tool call (`ToolCallStarted` →
//!   `ToolCallCompleted`) is open, the turn is never idle, however long the
//!   call is silent, up to [`OPEN_CALL_EXPIRY`] (a lost completion event must
//!   not hold the guard off for the rest of the turn). Those calls carry their own bounds: the harness per-model
//!   call ceiling and the tool's own `timeout_secs`, both under the harness
//!   turn ceiling. A silent provider retry (minutes) or a long shell build is
//!   in flight, not idle.
//! * A tool approval parks before `ToolCallStarted`, so it counts as silence.
//!   It is bounded by the approval TTL (600s by default), below the default
//!   idle window.
//!
//! What remains idle is time no other bound covers: session checkout before
//! the run, the persistence plumbing after it, or a harness that went silent
//! with no call open. The fixed total ceiling still exists (the backstop in
//! `agent::session_host::turn_backstop`) and still anchors the harness wind-down; it no
//! longer doubles as the hang detector.
//!
//! The clock is [`tokio::time::Instant`], so time the machine spends asleep is
//! not counted as idle, and tests can drive it with a paused runtime.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::agent::progress::AgentProgress;

/// Default idle window for a web-chat turn, in seconds.
pub(crate) const DEFAULT_TURN_IDLE_SECS: u64 = 900;

/// Environment override for the idle window, in seconds; `0` disables it.
pub(crate) const TURN_IDLE_ENV: &str = "OPENHUMAN_WEB_TURN_IDLE_SECS";

/// How long a call may stay open without its completion event before the
/// record stops counting it as in flight. Completion events can be dropped
/// under sustained backpressure; without a bound one lost `ToolCallCompleted`
/// would hold the idle guard off for the rest of the turn. Twice the harness's
/// per-model-call ceiling (900s). A long shell call is unaffected: its progress
/// heartbeat (every 15s) keeps the turn active on its own.
const OPEN_CALL_EXPIRY: Duration = Duration::from_secs(1_800);

/// Longest the watchdog sleeps between checks while a call is in flight. A
/// call that closes is followed by a full idle window measured from its close,
/// so this only bounds how late the watchdog notices that.
const IN_FLIGHT_RECHECK: Duration = Duration::from_secs(15);

/// Pure core of [`configured_idle_window`]: an absent or unparseable value
/// falls back to [`DEFAULT_TURN_IDLE_SECS`]; `0` disables the idle guard.
pub(crate) fn parse_idle_window(env_value: Option<&str>) -> Option<Duration> {
    let secs = env_value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_TURN_IDLE_SECS);
    (secs > 0).then(|| Duration::from_secs(secs))
}

/// The idle window in force for this process (env [`TURN_IDLE_ENV`]).
pub(crate) fn configured_idle_window() -> Option<Duration> {
    parse_idle_window(std::env::var(TURN_IDLE_ENV).ok().as_deref())
}

#[derive(Debug)]
struct State {
    last_event: Instant,
    /// Open tool calls by call id, with when each started.
    open_tool_calls: HashMap<String, Instant>,
    /// When the open parent model call started, if one is open.
    model_call_open: Option<Instant>,
    /// What the last event said the turn did, for the stop reply.
    last_step: Option<Cow<'static, str>>,
    turn_completed: bool,
    /// The config and reply locale the turn itself resolved, so a stop reply
    /// is written to the turn's own workspace and in its language.
    turn_config: Option<Arc<crate::config::Config>>,
    locale: Option<String>,
}

/// Where a turn was when it went silent, for the reply that closes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StuckPoint {
    /// No progress event at all: the turn never got past session setup.
    BeforeRun,
    /// The agent loop was between steps; `after` names its last step.
    BetweenSteps { after: String },
    /// The agent loop had finished; the reply was being saved or delivered.
    AfterRun,
}

impl StuckPoint {
    /// One plain-language clause naming where the turn stopped.
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::BeforeRun => {
                "before the agent started: the turn never got past setting up the chat \
                 session"
                    .to_string()
            }
            Self::BetweenSteps { after } => {
                format!("between steps: {after}, and the agent's next step never started")
            }
            Self::AfterRun => {
                "after the agent finished, while its reply was being saved".to_string()
            }
        }
    }
}

/// The step an event reports, as a clause for [`StuckPoint::BetweenSteps`].
/// `None` for bookkeeping (cost, usage, captured content) that says nothing
/// about what the turn was doing.
fn step_of(event: &AgentProgress) -> Option<Cow<'static, str>> {
    Some(match event {
        AgentProgress::TurnStarted => "the turn started".into(),
        AgentProgress::IterationStarted { .. } => "a model call started".into(),
        AgentProgress::ModelCallCompleted {
            subagent_task_id: None,
            ..
        } => "the model replied".into(),
        AgentProgress::ToolCallStarted { tool_name, .. } => {
            format!("the `{tool_name}` tool started").into()
        }
        AgentProgress::ToolCallCompleted {
            tool_name, success, ..
        } => {
            let outcome = if *success { "finished" } else { "failed" };
            format!("the `{tool_name}` tool {outcome}").into()
        }
        AgentProgress::TextDelta { .. } | AgentProgress::ThinkingDelta { .. } => {
            "the model was writing".into()
        }
        AgentProgress::ToolCallProgress { .. } => "a tool reported progress".into(),
        AgentProgress::SubagentSpawned { .. }
        | AgentProgress::SubagentCompleted { .. }
        | AgentProgress::SubagentFailed { .. }
        | AgentProgress::SubagentAwaitingUser { .. } => "a sub-agent reported progress".into(),
        _ => return None,
    })
}

/// Shared activity record for one turn. Cheap to clone; every clone sees the
/// same state.
#[derive(Clone, Debug)]
pub(crate) struct TurnActivity {
    state: Arc<Mutex<State>>,
}

impl TurnActivity {
    /// A record whose idle clock starts now, with nothing in flight. The turn
    /// emits no progress before its session is checked out, so that phase is
    /// measured from here.
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                last_event: Instant::now(),
                open_tool_calls: HashMap::new(),
                model_call_open: None,
                last_step: None,
                turn_completed: false,
                turn_config: None,
                locale: None,
            })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Fold one progress event into the record.
    ///
    /// Completion events can be lost, so later events also close what they
    /// prove finished. The agent loop runs model call, then that call's tool
    /// round, then the next model call: a `ToolCallStarted` means the model
    /// call that requested it returned, and an `IterationStarted` means the
    /// previous tool round finished.
    pub(crate) fn observe(&self, event: &AgentProgress) {
        let now = Instant::now();
        let mut state = self.lock();
        state.last_event = now;
        if let Some(step) = step_of(event) {
            state.last_step = Some(step);
        }
        match event {
            AgentProgress::IterationStarted { .. } => {
                state.open_tool_calls.clear();
                state.model_call_open = Some(now);
            }
            AgentProgress::ModelCallCompleted {
                subagent_task_id: None,
                ..
            } => state.model_call_open = None,
            AgentProgress::ToolCallStarted { call_id, .. } => {
                state.model_call_open = None;
                state.open_tool_calls.insert(call_id.clone(), now);
            }
            AgentProgress::ToolCallCompleted { call_id, .. } => {
                state.open_tool_calls.remove(call_id);
            }
            AgentProgress::TurnCompleted { .. } => {
                state.open_tool_calls.clear();
                state.model_call_open = None;
                state.turn_completed = true;
            }
            _ => {}
        }
    }

    /// How long the turn has been idle as of `now`, or `None` while a model or
    /// tool call is in flight. A call open longer than [`OPEN_CALL_EXPIRY`] no
    /// longer counts: its completion event was most likely lost.
    pub(crate) fn idle_for(&self, now: Instant) -> Option<Duration> {
        let state = self.lock();
        let live = |started: &Instant| now.saturating_duration_since(*started) < OPEN_CALL_EXPIRY;
        if state.model_call_open.as_ref().is_some_and(live)
            || state.open_tool_calls.values().any(live)
        {
            return None;
        }
        Some(now.saturating_duration_since(state.last_event))
    }
}

impl TurnActivity {
    /// Record the config and reply locale the turn resolved. A stop reply
    /// uses these rather than whatever profile is active when it fires.
    pub(crate) fn remember_turn(&self, config: Arc<crate::config::Config>, locale: Option<String>) {
        let mut state = self.lock();
        state.turn_config = Some(config);
        state.locale = locale;
    }

    /// The config and locale recorded by [`Self::remember_turn`], if the turn
    /// got that far.
    pub(crate) fn turn_context(&self) -> (Option<Arc<crate::config::Config>>, Option<String>) {
        let state = self.lock();
        (state.turn_config.clone(), state.locale.clone())
    }

    /// Where the turn was when its last event arrived.
    pub(crate) fn stuck_point(&self) -> StuckPoint {
        let state = self.lock();
        if state.turn_completed {
            return StuckPoint::AfterRun;
        }
        match &state.last_step {
            None => StuckPoint::BeforeRun,
            Some(step) => StuckPoint::BetweenSteps {
                after: step.to_string(),
            },
        }
    }
}

/// Resolve once the turn has been idle for `window`, returning the idle time
/// observed. Never resolves while the turn keeps emitting progress or keeps a
/// call in flight.
pub(crate) async fn idle_watchdog(activity: TurnActivity, window: Duration) -> Duration {
    loop {
        let wait = match activity.idle_for(Instant::now()) {
            Some(idle) if idle >= window => return idle,
            Some(idle) => window - idle,
            None => IN_FLIGHT_RECHECK.min(window),
        };
        tokio::time::sleep(wait).await;
    }
}

/// Interpose `activity` on a turn's progress stream: every event the agent
/// sends is folded into the record, then forwarded in order to the returned
/// receiver (the progress bridge). The forwarder only records and forwards, so
/// it drains the agent-side channel faster than the bridge, which publishes.
/// It ends when the agent's sender drops, closing the bridge's stream in turn.
pub(crate) fn tap(
    ctx: &crate::core::runtime::DispatchCtx,
    activity: TurnActivity,
    mut agent_rx: mpsc::Receiver<AgentProgress>,
    capacity: usize,
) -> mpsc::Receiver<AgentProgress> {
    let (bridge_tx, bridge_rx) = mpsc::channel(capacity);
    ctx.spawn(async move {
        while let Some(event) = agent_rx.recv().await {
            activity.observe(&event);
            if bridge_tx.send(event).await.is_err() {
                break;
            }
        }
    });
    bridge_rx
}

#[cfg(test)]
#[path = "turn_activity_tests.rs"]
mod tests;
