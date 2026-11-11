//! Per-run streaming logs for `skills_run`.
//!
//! Each run writes a human-readable trace to a host-owned storage stream: a
//! header (skill, inputs, task prompt), one line per agent step (tool calls +
//! results, sub-agent lifecycle, iteration boundaries) streamed live off the
//! agent's [`AgentProgress`] channel, then a footer (status, duration, final
//! output). The historical `log_path` field carries an opaque stream id.
//!
//! The `skills.runs.` stream prefix keeps run logs separate from skill
//! definitions and other host streams.
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc::Receiver;
use tokio_util::sync::CancellationToken;

use crate::progress::AgentProgress;
use crate::storage::StorageError;

pub use tinyskills::run_log::{detect_repeated_line, RunLogSlice, RunOutcome};
use tinyskills::run_log::{parse_runs, parse_terminal_outcome, short, truncate};

type RunLogStore = tinyskills_drivers::StreamRunLogStore;

/// One host-visible run. `log_path` is an opaque stream identifier retained
/// under the historical JSON field name for clients that tail a run.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ScannedRun {
    pub run_id: String,
    pub workflow_id: String,
    pub started: String,
    pub status: String,
    pub duration_ms: Option<u64>,
    pub finished: Option<String>,
    pub log_path: String,
}

fn run_log_store(workspace: &Path) -> anyhow::Result<RunLogStore> {
    let opened = crate::storage::local::open("skills::run_log", workspace)?;
    let storage = opened.scoped()?;
    let prepared = storage.clone();
    let scope = opened.scope.to_storage_scope();
    crate::storage::block_on(async move {
        crate::storage::boot::prepare_scope(&prepared, &scope).await?;
        Ok(())
    })?;
    Ok(RunLogStore::new(Arc::clone(storage.streams())))
}

fn stream_id(path: &Path) -> anyhow::Result<&str> {
    let id = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("run-log stream id is not UTF-8"))?;
    if !id.starts_with("skills.runs.") {
        anyhow::bail!("invalid run-log stream id");
    }
    Ok(id)
}

fn io_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

fn read_full(store: &RunLogStore, id: &str) -> Result<String, io::Error> {
    let mut text = String::new();
    let mut offset = 0;
    loop {
        let store = store.clone();
        let id = id.to_owned();
        let page = crate::storage::block_on(async move {
            store
                .read_slice(&id, offset, 64 * 1024)
                .await
                .map_err(|error| StorageError::backend(error.to_string()))
        })
        .map_err(io_error)?;
        offset = page.offset;
        text.push_str(&page.content);
        if page.eof || page.bytes_read == 0 {
            return Ok(text);
        }
    }
}

/// The host's old log-directory surface is retained only to build a stable
/// opaque stream identifier; no filesystem directory is created or read.
pub fn runs_dir(workspace: &Path) -> PathBuf {
    workspace.join("skills").join(".runs")
}

/// Find a run's opaque stream identifier by its run id.
pub fn find_run_log_path(workspace: &Path, run_id: &str) -> Option<PathBuf> {
    scan_runs(workspace, None, usize::MAX)
        .into_iter()
        .find(|run| run.run_id == run_id)
        .map(|run| PathBuf::from(run.log_path))
}

/// Read the full terminal outcome for a persisted run stream.
pub fn read_terminal_outcome(workspace: &Path, path: &Path) -> Option<RunOutcome> {
    let id = stream_id(path).ok()?;
    let store = run_log_store(workspace).ok()?;
    let text = read_full(&store, id).ok()?;
    parse_terminal_outcome(&text)
}

/// Scan persisted run streams, preserving the historical newest-first order.
pub fn scan_runs(workspace: &Path, workflow_id: Option<&str>, limit: usize) -> Vec<ScannedRun> {
    let store = match run_log_store(workspace) {
        Ok(store) => store,
        Err(error) => {
            log::warn!("[skills::run_log] could not open run-log storage: {error:#}");
            return Vec::new();
        }
    };
    let runs = match crate::storage::block_on(async move {
        store
            .scan(None, usize::MAX)
            .await
            .map_err(|error| StorageError::backend(error.to_string()))
    }) {
        Ok(runs) => runs,
        Err(error) => {
            log::warn!("[skills::run_log] could not scan run logs: {error}");
            return Vec::new();
        }
    };
    let store = match run_log_store(workspace) {
        Ok(store) => store,
        Err(error) => {
            log::warn!("[skills::run_log] could not reopen run-log storage: {error:#}");
            return Vec::new();
        }
    };
    let entries = runs
        .into_iter()
        .filter_map(|run| {
            let text = read_full(&store, &run.stream_id).ok()?;
            Some((run.stream_id, text))
        })
        .collect::<Vec<_>>();
    parse_runs(entries, workflow_id, limit)
        .into_iter()
        .map(|run| ScannedRun {
            run_id: run.run_id,
            workflow_id: run.workflow_id,
            started: run.started,
            status: run.status,
            duration_ms: run.duration_ms,
            finished: run.finished,
            log_path: run.stream_id,
        })
        .collect()
}

/// Read a byte-offset slice from a persisted run stream.
pub fn read_run_log_slice(
    workspace: &Path,
    path: &Path,
    offset: u64,
    max_bytes: usize,
) -> io::Result<RunLogSlice> {
    let id = stream_id(path).map_err(io_error)?.to_owned();
    let store = run_log_store(workspace).map_err(io_error)?;
    crate::storage::block_on(async move {
        store
            .read_slice(&id, offset, max_bytes)
            .await
            .map_err(|error| StorageError::backend(error.to_string()))
    })
    .map_err(io_error)
}

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
    let _ = workspace;
    PathBuf::from(format!(
        "skills.runs.{}.{}.{}",
        tinyskills::run_log::sanitize(workflow_id),
        tinyskills::run_log::sanitize(&ts),
        tinyskills::run_log::sanitize(run_id)
    ))
}

/// Write the run header, stamped with the host clock.
pub async fn write_header(
    workspace: &Path,
    path: &Path,
    workflow_id: &str,
    run_id: &str,
    inputs: &Value,
    task_prompt: &str,
) -> std::io::Result<()> {
    let started = chrono::Utc::now().to_rfc3339();
    let id = stream_id(path).map_err(io_error)?;
    let timestamp = id
        .strip_prefix("skills.runs.")
        .and_then(|tail| tail.split('.').nth(1))
        .ok_or_else(|| io_error("invalid run-log stream id"))?;
    let store = run_log_store(workspace).map_err(io_error)?;
    let workflow_id = workflow_id.to_owned();
    let run_id = run_id.to_owned();
    let timestamp = timestamp.to_owned();
    let inputs = inputs.clone();
    let task_prompt = task_prompt.to_owned();
    let created = crate::storage::block_on(async move {
        store
            .start(
                &workflow_id,
                &run_id,
                &timestamp,
                &started,
                &inputs,
                &task_prompt,
            )
            .await
            .map_err(|error| StorageError::backend(error.to_string()))
    })
    .map_err(io_error)?;
    if created != id {
        return Err(io_error("run-log stream id did not match the requested id"));
    }
    Ok(())
}

/// Write the final footer, stamped with the host clock.
pub async fn write_footer(
    workspace: &Path,
    path: &Path,
    status: &str,
    elapsed_ms: u64,
    output: &str,
) -> std::io::Result<()> {
    let finished = chrono::Utc::now().to_rfc3339();
    let id = stream_id(path).map_err(io_error)?.to_owned();
    let store = run_log_store(workspace).map_err(io_error)?;
    let status = status.to_owned();
    let output = output.to_owned();
    crate::storage::block_on(async move {
        store
            .finish(&id, &status, elapsed_ms, &finished, &output)
            .await
            .map_err(|error| StorageError::backend(error.to_string()))
    })
    .map_err(io_error)
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
        AgentProgress::ContextCompacting => "· summarizing earlier conversation".to_string(),
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
    workspace: PathBuf,
    path: PathBuf,
) -> Option<tinyagents_harness::turn_stop::TurnStop> {
    let store = match run_log_store(&workspace) {
        Ok(store) => store,
        Err(error) => {
            log::warn!("[skills::run_log] could not open run-log storage: {error:#}");
            return None;
        }
    };
    let id = match stream_id(&path) {
        Ok(id) => id.to_owned(),
        Err(error) => {
            log::warn!("[skills::run_log] invalid run-log stream: {error:#}");
            return None;
        }
    };
    let mut stop = None;
    while let Some(ev) = rx.recv().await {
        if let Some(line) = format_event(&ev) {
            if let Err(error) = store.append_line(&id, &line).await {
                log::warn!("[skills::run_log] append failed: {error}");
            }
        }
        if let AgentProgress::TurnCompleted { stop: Some(s), .. } = &ev {
            stop = Some(s.clone());
        }
    }
    stop
}

/// [`drain_to_log`] on its own task, acting for `ctx`. The log store resolves
/// its storage scope from the acting context, so a bare spawn would write the
/// steps under the default scope (or, in SaaS, fail closed) while the header
/// sits under the dispatching tenant's.
pub fn spawn_drain_to_log(
    ctx: &crate::core::runtime::DispatchCtx,
    rx: Receiver<AgentProgress>,
    workspace: PathBuf,
    path: PathBuf,
) -> tokio::task::JoinHandle<Option<tinyagents_harness::turn_stop::TurnStop>> {
    ctx.spawn(drain_to_log(rx, workspace, path))
}

#[cfg(test)]
#[path = "run_log_tests.rs"]
mod tests;
