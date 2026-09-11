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
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::Receiver;
use tokio_util::sync::CancellationToken;

use crate::openhuman::agent::progress::AgentProgress;

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

/// `<workspace>/skills/.runs`.
pub fn runs_dir(workspace: &Path) -> PathBuf {
    workspace.join("skills").join(".runs")
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn short(s: &str) -> &str {
    s.get(..8).unwrap_or(s)
}

/// `<runs_dir>/<skill>_<UTC ts>_<short run id>.log`.
pub fn run_log_path(workspace: &Path, workflow_id: &str, run_id: &str) -> PathBuf {
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    runs_dir(workspace).join(format!(
        "{}_{}_{}.log",
        sanitize(workflow_id),
        ts,
        sanitize(short(run_id))
    ))
}

async fn append(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.ok();
    }
    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    f.write_all(line.as_bytes()).await?;
    if !line.ends_with('\n') {
        f.write_all(b"\n").await?;
    }
    f.flush().await
}

fn truncate(s: &str, n: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n).collect::<String>())
    } else {
        s
    }
}

/// Write the run header (skill, inputs, the resolved task prompt).
pub async fn write_header(
    path: &Path,
    workflow_id: &str,
    run_id: &str,
    inputs: &Value,
    task_prompt: &str,
) -> std::io::Result<()> {
    write_header_with_profile(path, workflow_id, run_id, inputs, task_prompt, None).await
}

/// Write a run header attributed to the active profile. Profile-less callers
/// retain the legacy header format through [`write_header`].
pub async fn write_header_with_profile(
    path: &Path,
    workflow_id: &str,
    run_id: &str,
    inputs: &Value,
    task_prompt: &str,
    profile_id: Option<&str>,
) -> std::io::Result<()> {
    let profile_line = profile_id
        .map(|id| format!("profile_id: {id}\n"))
        .unwrap_or_default();
    let header = format!(
        "==== workflow_run: {skill} ====\n\
         run_id : {run}\n\
         {profile_line}\
         started: {start} UTC\n\
         inputs : {inputs}\n\n\
         --- task prompt ---\n{prompt}\n\n\
         --- steps ---",
        skill = workflow_id,
        run = run_id,
        start = chrono::Utc::now().to_rfc3339(),
        inputs = serde_json::to_string(inputs).unwrap_or_default(),
        prompt = task_prompt,
    );
    append(path, &header).await
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
        AgentProgress::TurnCompleted { iterations } => {
            format!("turn completed ({iterations} iterations)")
        }
        // Noisy / non-step events — skipped (the final text is in the footer).
        AgentProgress::TextDelta { .. }
        | AgentProgress::ThinkingDelta { .. }
        | AgentProgress::ToolCallArgsDelta { .. }
        | AgentProgress::TurnCostUpdated { .. }
        | AgentProgress::ModelCallCompleted { .. }
        | AgentProgress::TaskBoardUpdated { .. }
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
pub async fn drain_to_log(mut rx: Receiver<AgentProgress>, path: PathBuf) {
    while let Some(ev) = rx.recv().await {
        if let Some(line) = format_event(&ev) {
            let _ = append(&path, &line).await;
        }
    }
}

/// Detect the degenerate "model emitted the same paragraph many times in one
/// generation" final-response failure mode we keep seeing on autonomous runs
/// (e.g. `"Now I understand the structure..." × 23`, `"Good, the repo is
/// cloned. Let me narrow down..." × 8`). When this fires we don't want the
/// autonomous-skill path to mark the run `DONE` and have callers treat the
/// degenerate text as a real result — we want it surfaced as `DEGENERATE` with
/// the offending line attached, so the caller can retry / fail loud.
///
/// Splits on line boundaries (each repeat we've observed lands on its own
/// line or paragraph), trims, counts non-trivial lines (`>= min_len` chars),
/// and returns the most-repeated line if its count reaches `min_count`.
pub fn detect_repeated_line(
    text: &str,
    min_len: usize,
    min_count: usize,
) -> Option<(String, usize)> {
    use std::collections::HashMap;
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for line in text.lines() {
        let t = line.trim();
        if t.len() >= min_len {
            *counts.entry(t).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .filter(|(_, c)| *c >= min_count)
        .max_by_key(|(_, c)| *c)
        .map(|(line, count)| (line.to_string(), count))
}

/// One run extracted from a `.runs/<skill>_<utc>_<run>.log` file. Built by
/// [`scan_runs`] for the `openhuman.skills_recent_runs` RPC + the Skills
/// Runner panel's "Recent runs" section. Status is `RUNNING` until the
/// footer block (`--- result ---` + `status: …` + `duration: … ms`) lands.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct ScannedRun {
    pub run_id: String,
    pub workflow_id: String,
    /// Profile that launched the run. `None` denotes a legacy/profile-less run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    /// Header `started:` timestamp (RFC3339); empty if header was malformed.
    pub started: String,
    /// `"DONE"` / `"DEGENERATE"` / `"FAILED"` / `"RUNNING"` (running ⇔ no footer yet).
    pub status: String,
    /// Footer `duration: <ms> ms`, parsed; `None` while running.
    pub duration_ms: Option<u64>,
    /// Footer `finished:` timestamp; `None` while running.
    pub finished: Option<String>,
    /// Absolute path to the streaming log file — what the FE shows for
    /// "view full log" or future tail-streaming.
    pub log_path: String,
}

/// Scan `<workspace>/skills/.runs/` for run-log files, parse their header +
/// footer, and return a vec sorted by `started` *descending* (most-recent
/// first). When `workflow_id` is `Some(_)`, only entries whose header
/// `workflow_id` matches are returned. `limit` caps the result (post-filter,
/// post-sort) so the panel can render a short list cheaply. Malformed
/// files are skipped silently — never blocks the response.
pub fn scan_runs(workspace: &Path, workflow_id: Option<&str>, limit: usize) -> Vec<ScannedRun> {
    let dir = runs_dir(workspace);
    let mut runs: Vec<ScannedRun> = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return runs;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".log") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut sid = String::new();
        let mut rid = String::new();
        let mut started = String::new();
        let mut profile_id: Option<String> = None;
        let mut status = String::from("RUNNING");
        let mut duration_ms: Option<u64> = None;
        let mut finished: Option<String> = None;
        let mut seen_result = false;
        // The on-disk format from `write_header` / `write_footer` is
        // label-then-colon-then-value, with the labels right-padded for
        // visual alignment in the log — e.g. `status  : DONE`,
        // `duration: 617236 ms`, `run_id : <uuid>`. Splitting on the FIRST
        // `:` and trimming both halves is robust to that padding without
        // hand-tracking each label's exact whitespace.
        for line in text.lines() {
            if line.starts_with("==== workflow_run:") {
                // Header banner: `==== workflow_run: <id> ====`
                sid = line
                    .trim_start_matches("==== workflow_run:")
                    .trim()
                    .trim_end_matches('=')
                    .trim()
                    .to_string();
                continue;
            }
            if line.starts_with("--- result ---") {
                seen_result = true;
                continue;
            }
            let Some((label_raw, value_raw)) = line.split_once(':') else {
                continue;
            };
            let label = label_raw.trim();
            let value = value_raw.trim();
            match (label, seen_result) {
                // Header fields (before --- result ---)
                ("run_id", false) => rid = value.to_string(),
                ("profile_id", false) => profile_id = Some(value.to_string()),
                ("started", false) => started = value.to_string(),
                // Footer fields (after --- result ---)
                ("status", true) => status = value.to_string(),
                ("duration", true) => {
                    // Format: "<n> ms"
                    let num = value.trim_end_matches(" ms").trim();
                    if let Ok(n) = num.parse::<u64>() {
                        duration_ms = Some(n);
                    }
                }
                ("finished", true) => {
                    finished = Some(value.trim_end_matches(" UTC").trim().to_string());
                }
                _ => {}
            }
        }
        if sid.is_empty() || rid.is_empty() {
            // Malformed header — skip rather than show a half-row.
            continue;
        }
        if let Some(want) = workflow_id {
            if sid != want {
                continue;
            }
        }
        runs.push(ScannedRun {
            run_id: rid,
            workflow_id: sid,
            profile_id,
            started,
            status,
            duration_ms,
            finished,
            log_path: path.to_string_lossy().into_owned(),
        });
    }
    // Sort most-recent first by `started` (RFC3339 sorts lexicographically).
    runs.sort_by(|a, b| b.started.cmp(&a.started));
    runs.truncate(limit);
    runs
}

/// Look up the on-disk log path for a given `run_id` by scanning the
/// `<workspace>/skills/.runs/` directory. Used by
/// `openhuman.skills_read_run_log` to resolve a stable id back to a path
/// without trusting the caller to send one (no path-traversal surface).
pub fn find_run_log_path(workspace: &Path, run_id: &str) -> Option<PathBuf> {
    if run_id.is_empty() {
        return None;
    }
    let dir = runs_dir(workspace);
    let entries = std::fs::read_dir(&dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".log") {
            continue;
        }
        // File names are `<skill>_<utc>_<run-id-prefix>.log`. The run-id
        // prefix is the first 8 chars of the uuid (see
        // `runs_dir`/`run_log_path` + `short` helper). Match against the
        // prefix to avoid having to read the file's header.
        let short = run_id.get(..8).unwrap_or(run_id);
        if name.contains(&format!("_{short}.log")) {
            return Some(path);
        }
    }
    None
}

/// Terminal outcome of a finished run, parsed from the `--- result ---`
/// footer: the status word (`DONE` / `DEGENERATE` / `FAILED`) and the
/// final output body that follows it. Used by `run_workflow` /
/// `await_workflow` to hand the spawned run's result straight back to the
/// orchestrator instead of making it scrape the log itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub status: String,
    pub output: String,
}

/// Read a run-log file and, if the footer has landed, return the terminal
/// status + final output body. Returns `None` while the run is still
/// `RUNNING` (no footer yet) or the file is unreadable — callers poll on
/// `None`. Mirrors the on-disk layout written by [`write_footer`]:
///
/// ```text
/// --- result ---
/// status  : DONE
/// duration: 1234 ms
/// finished: <rfc3339> UTC
///
/// <output body…>
/// ```
pub fn read_terminal_outcome(path: &Path) -> Option<RunOutcome> {
    let text = std::fs::read_to_string(path).ok()?;
    let marker = "--- result ---";
    let idx = text.find(marker)?;
    let after = &text[idx + marker.len()..];
    let mut status = String::new();
    let mut saw_finished = false;
    let mut lines = after.lines();
    // Consume the footer header lines (status/duration/finished); the first
    // blank line after `finished:` separates them from the output body.
    for line in lines.by_ref() {
        let Some((label, value)) = line.split_once(':') else {
            // A line without a colon before `finished:` means a malformed
            // footer — bail rather than mis-slice the body.
            if line.trim().is_empty() {
                continue;
            }
            break;
        };
        match label.trim() {
            "status" => status = value.trim().to_string(),
            "finished" => {
                saw_finished = true;
                break;
            }
            _ => {}
        }
    }
    // Require BOTH `status:` and the closing `finished:` line — write_footer
    // emits `finished:` last, so its absence means we raced a partially
    // written (or malformed) footer and the run isn't actually terminal yet.
    if status.is_empty() || !saw_finished {
        return None;
    }
    // Everything past the footer header is the final output body.
    let output = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    Some(RunOutcome { status, output })
}

/// Read a slice of a run log file. Returns the bytes from `offset`
/// forward, capped at `max_bytes`, plus `eof` (true if we hit end-of-
/// file) and a flag indicating whether the `--- result ---` footer is
/// present in the file as a whole (so the FE can stop polling). Used by
/// `openhuman.skills_read_run_log` for the chat-style log viewer's
/// scroll + tail behaviour.
pub fn read_run_log_slice(
    path: &Path,
    offset: u64,
    max_bytes: usize,
) -> std::io::Result<RunLogSlice> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let file_size = f.metadata()?.len();
    if offset >= file_size {
        // No new bytes. Still return a (cheap) check for footer presence
        // so the FE knows whether to keep polling.
        let complete = file_has_footer(path)?;
        return Ok(RunLogSlice {
            offset,
            bytes_read: 0,
            content: String::new(),
            eof: true,
            complete,
        });
    }
    f.seek(SeekFrom::Start(offset))?;
    let want = ((file_size - offset).min(max_bytes as u64)) as usize;
    let mut buf = vec![0u8; want];
    f.read_exact(&mut buf)?;
    let content = String::from_utf8_lossy(&buf).into_owned();
    let bytes_read = buf.len() as u64;
    let new_offset = offset + bytes_read;
    let eof = new_offset >= file_size;
    let complete = if eof {
        // If we read to EOF, the slice itself tells us if the footer
        // landed in our current chunk — otherwise re-scan from disk.
        content.contains("\n--- result ---\n")
            || content.starts_with("--- result ---\n")
            || file_has_footer(path)?
    } else {
        // Mid-file read — cheap re-scan to know if we should keep polling.
        file_has_footer(path)?
    };
    Ok(RunLogSlice {
        offset: new_offset,
        bytes_read,
        content,
        eof,
        complete,
    })
}

/// One slice of a run log file. `offset` is the *new* read cursor (the
/// FE uses it as the next call's `offset` so successive reads tail
/// cleanly). `complete` is true once the run footer landed — the FE can
/// then stop the polling timer.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RunLogSlice {
    pub offset: u64,
    pub bytes_read: u64,
    pub content: String,
    pub eof: bool,
    pub complete: bool,
}

/// Cheap check for whether `path` contains the `--- result ---` footer
/// anywhere. Reads the file once. Used to decide if the FE should keep
/// polling.
fn file_has_footer(path: &Path) -> std::io::Result<bool> {
    let text = std::fs::read_to_string(path)?;
    Ok(text.contains("\n--- result ---\n"))
}

/// Final footer: status, duration, and the agent's final output text.
pub async fn write_footer(
    path: &Path,
    status: &str,
    elapsed_ms: u64,
    output: &str,
) -> std::io::Result<()> {
    let footer = format!(
        "\n--- result ---\n\
         status  : {status}\n\
         duration: {elapsed_ms} ms\n\
         finished: {fin} UTC\n\n{output}\n",
        fin = chrono::Utc::now().to_rfc3339(),
    );
    append(path, &footer).await
}

#[cfg(test)]
#[path = "run_log_tests.rs"]
mod tests;
