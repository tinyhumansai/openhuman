//! Per-run streaming logs for `skills_run`.
//!
//! Each run writes a human-readable trace to
//! `<workspace>/skills/.runs/<skill>_<UTC-ts>_<run>.log`: a header (skill,
//! inputs, task prompt), one line per agent step (tool calls + results,
//! sub-agent lifecycle, iteration boundaries) streamed live off the agent's
//! [`AgentProgress`] channel, then a footer (status, duration, final output).
//!
//! `.runs` is a sibling of the runtime skill *definitions* (`<workspace>/
//! skills/<id>/`) so run logs never collide with a skill-id directory.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde_json::Value;
use tokio::sync::mpsc::Receiver;
use tokio_util::sync::CancellationToken;

use crate::agent::progress::AgentProgress;

// Layout, writers-by-timestamp and readers live in `tinyskills::run_log`;
// the host keeps event formatting, the cancel registry and the drain.
pub use tinyskills::run_log::{
    detect_repeated_line, find_run_log_path, read_run_log_slice, read_terminal_outcome, runs_dir,
    scan_runs, RunLogSlice, RunOutcome, ScannedRun,
};
use tinyskills::run_log::{short, truncate};

/// Registry of in-flight workflow runs → their cancellation token. A run
/// registers itself before executing and removes itself when it finishes; the
/// `skills_cancel` RPC looks a run up by id and fires its token, which the
/// run's `tokio::select!` observes to stop the agent and write a `CANCELLED`
/// footer.
static RUN_CANCELS: Lazy<Mutex<HashMap<String, CancellationToken>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Register a fresh cancellation token for `run_id` and return it. The caller
/// passes the returned token into its run loop's `select!`.
pub fn register_run_cancel(run_id: &str) -> CancellationToken {
    let token = CancellationToken::new();
    let live = {
        let mut map = RUN_CANCELS.lock().unwrap_or_else(|e| e.into_inner());
        map.insert(run_id.to_string(), token.clone());
        map.len()
    };
    log::debug!("[workflows::run-cancel] registered run_id={run_id} (live={live})");
    token
}

/// Drop the registry entry for `run_id` (call once the run is fully done).
pub fn unregister_run_cancel(run_id: &str) {
    let (existed, live) = {
        let mut map = RUN_CANCELS.lock().unwrap_or_else(|e| e.into_inner());
        let existed = map.remove(run_id).is_some();
        (existed, map.len())
    };
    log::debug!(
        "[workflows::run-cancel] unregistered run_id={run_id} (existed={existed}, live={live})"
    );
}

/// Signal cancellation for a running run. Returns `true` if a live run with
/// this id was found and signalled, `false` if it's unknown (already finished
/// or never existed).
pub fn cancel_run(run_id: &str) -> bool {
    let found = match RUN_CANCELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(run_id)
    {
        Some(token) => {
            token.cancel();
            true
        }
        None => false,
    };
    log::debug!("[workflows::run-cancel] cancel requested run_id={run_id} (found={found})");
    found
}

/// `<runs_dir>/<skill>_<UTC ts>_<short run id>.log`, stamped with the host clock.
pub fn run_log_path(workspace: &Path, workflow_id: &str, run_id: &str) -> PathBuf {
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    tinyskills::run_log::run_log_path(workspace, workflow_id, run_id, &ts)
}

/// Write the run header, stamped with the host clock.
pub async fn write_header(
    path: &Path,
    workflow_id: &str,
    run_id: &str,
    inputs: &Value,
    task_prompt: &str,
) -> std::io::Result<()> {
    let started = chrono::Utc::now().to_rfc3339();
    tinyskills::run_log::write_header(path, workflow_id, run_id, &started, inputs, task_prompt)
}

/// Write the final footer, stamped with the host clock.
pub async fn write_footer(
    path: &Path,
    status: &str,
    elapsed_ms: u64,
    output: &str,
) -> std::io::Result<()> {
    let finished = chrono::Utc::now().to_rfc3339();
    tinyskills::run_log::write_footer(path, status, elapsed_ms, &finished, output)
}

/// One log line for a step, or `None` for events too noisy to log per-event
/// (token / argument deltas, cost ticks — the final text lands in the footer).
pub fn format_event(ev: &AgentProgress) -> Option<String> {
    let line = match ev {
        AgentProgress::TurnStarted => "turn started".to_string(),
        AgentProgress::IterationStarted {
            iteration,
            max_iterations,
        } => format!("· iteration {iteration}/{max_iterations}"),
        AgentProgress::ToolCallStarted {
            tool_name,
            arguments,
            iteration,
            ..
        } => format!(
            "[it {iteration}] tool {tool_name}({})",
            truncate(&arguments.to_string(), 200)
        ),
        AgentProgress::ToolCallCompleted {
            tool_name,
            success,
            output_chars,
            elapsed_ms,
            ..
        } => format!(
            "        ↳ {tool_name} {} ({output_chars} chars, {elapsed_ms} ms)",
            if *success { "ok" } else { "FAILED" }
        ),
        AgentProgress::SubagentSpawned {
            agent_id,
            task_id,
            prompt_chars,
            ..
        } => format!(
            "  ⮑ spawned subagent {agent_id} [{}] ({prompt_chars}-char prompt)",
            short(task_id)
        ),
        AgentProgress::SubagentToolCallStarted {
            agent_id,
            tool_name,
            ..
        } => format!("    [{agent_id}] tool {tool_name}"),
        AgentProgress::SubagentToolCallCompleted {
            agent_id,
            tool_name,
            success,
            elapsed_ms,
            ..
        } => format!(
            "    [{agent_id}] ↳ {tool_name} {} ({elapsed_ms} ms)",
            if *success { "ok" } else { "FAILED" }
        ),
        AgentProgress::SubagentCompleted {
            agent_id,
            elapsed_ms,
            iterations,
            ..
        } => format!("  ⮑ subagent {agent_id} done ({iterations} turns, {elapsed_ms} ms)"),
        AgentProgress::SubagentFailed {
            agent_id, error, ..
        } => format!("  ⮑ subagent {agent_id} FAILED: {}", truncate(error, 200)),
        AgentProgress::SubagentAwaitingUser {
            agent_id, question, ..
        } => format!(
            "  ⮑ subagent {agent_id} awaiting user: {}",
            truncate(question, 200)
        ),
        AgentProgress::TurnCompleted {
            iterations,
            stop: Some(stop),
        } => format!("turn {} ({iterations} iterations)", stop.status_message()),
        AgentProgress::TurnCompleted { iterations, .. } => {
            format!("turn completed ({iterations} iterations)")
        }
        // Noisy / non-step events — skipped (the final text is in the footer).
        AgentProgress::TextDelta { .. }
        | AgentProgress::ThinkingDelta { .. }
        | AgentProgress::ToolCallArgsDelta { .. }
        | AgentProgress::ToolCallProgress { .. }
        | AgentProgress::TurnCostUpdated { .. }
        | AgentProgress::ModelCallCompleted { .. }
        | AgentProgress::SubagentTextDelta { .. }
        | AgentProgress::SubagentThinkingDelta { .. }
        | AgentProgress::SubagentIterationStarted { .. }
        | AgentProgress::TurnContent { .. } => return None,
    };
    Some(format!(
        "{}  {}",
        chrono::Utc::now().format("%H:%M:%S%.3f"),
        line
    ))
}

/// Drain the progress channel to the log until the agent drops its sender.
/// Returns how the run's turn was stopped early, if its `TurnCompleted` said
/// so, so the caller can write a `STOPPED` footer instead of `DONE`.
pub async fn drain_to_log(
    mut rx: Receiver<AgentProgress>,
    path: PathBuf,
) -> Option<tinyagents_harness::turn_stop::TurnStop> {
    let mut stop = None;
    while let Some(ev) = rx.recv().await {
        if let Some(line) = format_event(&ev) {
            let _ = tinyskills::run_log::append_line(&path, &line);
        }
        if let AgentProgress::TurnCompleted { stop: Some(s), .. } = &ev {
            stop = Some(s.clone());
        }
    }
    stop
}

#[cfg(test)]
#[path = "run_log_tests.rs"]
mod tests;
