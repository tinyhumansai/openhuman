//! The shell commands a turn left running past their yield point.
//!
//! The `shell` tool reserves a [`Slot`] before it starts a command that may
//! yield, and the slot adopts the [`LiveShell`] if the command is still running
//! at its yield point; the model gets its id (`p1`, `p2`, …). The
//! `shell_process` tool waits on it, stops it or reports its status. One
//! registry belongs to one turn's run context, so dropping it at the end of the
//! turn stops every process still running (each [`LiveHandle`] stops its own on
//! drop). Every process also stops at its own runtime budget, which the turn
//! budget already clamps.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::shell_live::{LiveHandle, LiveRead, LiveShell, LiveState};

/// Processes a turn may leave running at once. A further command runs to its
/// end the usual way.
pub(crate) const MAX_RUNNING: usize = 4;

/// Consecutive waits with no new output after which the report suggests
/// stopping the process.
pub(crate) const QUIET_WAITS_NOTE: u32 = 3;

/// How a process ended, kept for later questions about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ended {
    Exited(i32),
    TimedOut,
    Stopped,
}

/// What a wait, stop or status call found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReportState {
    /// Still running.
    Running,
    /// Ended, and this answer is the first to say so. A `wait` or `kill`
    /// answer carries the rest of its output; a `status` answer leaves that
    /// output for the next `wait`.
    Ended(Ended),
    /// Had already ended, and its output was already returned.
    AlreadyEnded(Ended),
}

/// One answer about a process, for the tool to render.
#[derive(Clone, Debug)]
pub(crate) struct Report {
    pub(crate) id: String,
    pub(crate) command: String,
    /// Time since the command started.
    pub(crate) elapsed: Duration,
    /// The process's runtime budget.
    pub(crate) timeout: Duration,
    /// Output written since the previous answer.
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) state: ReportState,
    /// Consecutive waits, this one included, that brought no new output.
    pub(crate) quiet_waits: u32,
}

/// Called once with how the process ended and how long it ran, when the
/// registry learns of the end (or stops it at the end of the turn). The shell
/// tool records its audit entry here, so a yielded command is audited with its
/// real outcome.
pub(crate) type OnEnd = Box<dyn FnOnce(Ended, Duration) + Send>;

struct Entry {
    handle: LiveHandle,
    command: String,
    started: Instant,
    timeout: Duration,
    ended: Option<Ended>,
    /// Whether an answer has said the process ended.
    end_reported: bool,
    /// Output a `status` read that no answer has returned yet.
    pending_stdout: String,
    pending_stderr: String,
    quiet_waits: u32,
    on_end: Option<OnEnd>,
}

impl Drop for Entry {
    fn drop(&mut self) {
        // The turn ended with the process still running; its handle stops it.
        if let Some(on_end) = self.on_end.take() {
            on_end(self.ended.unwrap_or(Ended::Stopped), self.started.elapsed());
        }
    }
}

/// The running processes of one turn.
#[derive(Default)]
pub(crate) struct ShellProcesses {
    next: AtomicU32,
    entries: Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<Entry>>>>,
    /// Processes running or reserved: a slot counts from [`Self::reserve`].
    running: Mutex<usize>,
}

/// A registry is equal only to itself: two turns never share one.
impl PartialEq for ShellProcesses {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl Eq for ShellProcesses {}

/// A place for one more running process, held while its command starts. It
/// adopts the command if it is still running at its yield point; dropped
/// unused (the command finished first, or failed), it frees the place.
pub(crate) struct Slot {
    processes: Arc<ShellProcesses>,
    held: bool,
}

impl Slot {
    /// Take over a process still running at its yield point and name it.
    pub(crate) fn adopt(
        mut self,
        live: LiveShell,
        command: &str,
        started: Instant,
        timeout: Duration,
        on_end: Option<OnEnd>,
    ) -> String {
        self.held = false;
        let processes = &self.processes;
        let id = format!("p{}", processes.next.fetch_add(1, Ordering::SeqCst) + 1);
        processes.entries().insert(
            id.clone(),
            Arc::new(tokio::sync::Mutex::new(Entry {
                handle: live.handle,
                command: command.to_owned(),
                started,
                timeout,
                ended: None,
                end_reported: false,
                pending_stdout: String::new(),
                pending_stderr: String::new(),
                quiet_waits: 0,
                on_end,
            })),
        );
        tracing::debug!(process = %id, "[shell] process left running past its yield point");
        id
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        if self.held {
            let mut running = self.processes.running();
            *running = running.saturating_sub(1);
        }
    }
}

impl ShellProcesses {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Reserve a place for a command that may be left running, or `None`
    /// when the turn already holds [`MAX_RUNNING`]. Reserving before the
    /// command starts keeps parallel calls from all seeing room.
    pub(crate) fn reserve(self: &Arc<Self>) -> Option<Slot> {
        let mut running = self.running();
        if *running >= MAX_RUNNING {
            return None;
        }
        *running += 1;
        Some(Slot {
            processes: Arc::clone(self),
            held: true,
        })
    }

    /// Wait up to `wait` for new output or the end of the process.
    pub(crate) async fn wait(&self, id: &str, wait: Duration) -> Result<Report, String> {
        let entry = self.entry(id)?;
        let mut entry = entry.lock().await;
        if entry.ended.is_some() {
            return Ok(self.answer_ended(id, &mut entry));
        }
        let read = match entry.handle.read(wait).await {
            Ok(read) => read,
            Err(error) => {
                return Err(self.failed(&mut entry, format!("Could not read process {id}: {error}")))
            }
        };
        let (stdout, stderr) = take_pending(&mut entry, read.stdout, read.stderr);
        let quiet = stdout.is_empty() && stderr.is_empty();
        entry.quiet_waits = if quiet { entry.quiet_waits + 1 } else { 0 };
        let state = self.settle(&mut entry, read.state);
        returned(&mut entry, &state);
        Ok(report(id, &entry, stdout, stderr, state))
    }

    /// Stop the process and return what it wrote since the previous answer.
    /// A process that does not stop is reported as still running.
    pub(crate) async fn kill(&self, id: &str) -> Result<Report, String> {
        let entry = self.entry(id)?;
        let mut entry = entry.lock().await;
        if entry.ended.is_some() {
            return Ok(self.answer_ended(id, &mut entry));
        }
        tracing::debug!(process = %id, "[shell] stopping a running process on request");
        let read = match entry.handle.kill().await {
            Ok(read) => read,
            Err(error) => {
                return Err(self.failed(&mut entry, format!("Could not stop process {id}: {error}")))
            }
        };
        let (stdout, stderr) = take_pending(&mut entry, read.stdout, read.stderr);
        let state = self.settle(&mut entry, read.state);
        returned(&mut entry, &state);
        Ok(report(id, &entry, stdout, stderr, state))
    }

    /// Where the process is now, without its output: a read that returns at
    /// once refreshes its state, and what it read is kept for the next
    /// `wait`, so a process that has ended says so and frees its place.
    pub(crate) async fn status(&self, id: &str) -> Result<Report, String> {
        let entry = self.entry(id)?;
        let mut entry = entry.lock().await;
        if entry.ended.is_none() {
            let read = match entry.handle.read(Duration::ZERO).await {
                Ok(read) => read,
                Err(error) => {
                    let message = format!("Could not read process {id}: {error}");
                    return Err(self.failed(&mut entry, message));
                }
            };
            let LiveRead {
                stdout,
                stderr,
                state,
            } = read;
            entry.pending_stdout.push_str(&stdout);
            entry.pending_stderr.push_str(&stderr);
            self.settle(&mut entry, state);
        }
        let state = match entry.ended {
            None => ReportState::Running,
            Some(ended) if entry.end_reported => ReportState::AlreadyEnded(ended),
            Some(ended) => ReportState::Ended(ended),
        };
        Ok(report(id, &entry, String::new(), String::new(), state))
    }

    /// A read that failed. When the handle has already stopped the process and
    /// released it (TinyBox's output limit, a failed cleanup), the process is
    /// over: it frees its place, and later answers say it stopped. Otherwise
    /// it may still be running and keeps its place.
    fn failed(&self, entry: &mut Entry, message: String) -> String {
        if entry.handle.finished() {
            tracing::debug!("[shell] a process ended in a failed read; freeing its place");
            self.settle(entry, LiveState::Stopped);
            entry.end_reported = true;
        }
        message
    }

    /// The answer for a process already known to have ended: its last output
    /// the first time, "already ended" after that.
    fn answer_ended(&self, id: &str, entry: &mut Entry) -> Report {
        let ended = entry.ended.unwrap_or(Ended::Stopped);
        if entry.end_reported {
            return report(
                id,
                entry,
                String::new(),
                String::new(),
                ReportState::AlreadyEnded(ended),
            );
        }
        entry.end_reported = true;
        let (stdout, stderr) = take_pending(entry, String::new(), String::new());
        report(id, entry, stdout, stderr, ReportState::Ended(ended))
    }

    /// Record an ending, once: free the place and report the outcome. An
    /// answer that returns the process's last output then marks the end
    /// reported (`returned`).
    fn settle(&self, entry: &mut Entry, state: LiveState) -> ReportState {
        let ended = match state {
            LiveState::Running => return ReportState::Running,
            LiveState::Exited(code) => Ended::Exited(code),
            LiveState::TimedOut => Ended::TimedOut,
            LiveState::Stopped => Ended::Stopped,
        };
        if entry.ended.is_none() {
            entry.ended = Some(ended);
            let mut running = self.running();
            *running = running.saturating_sub(1);
            drop(running);
            if let Some(on_end) = entry.on_end.take() {
                on_end(ended, entry.started.elapsed());
            }
        }
        ReportState::Ended(ended)
    }

    fn entry(&self, id: &str) -> Result<Arc<tokio::sync::Mutex<Entry>>, String> {
        let entries = self.entries();
        entries.get(id.trim()).cloned().ok_or_else(|| {
            let known: Vec<&str> = entries.keys().map(String::as_str).collect();
            if known.is_empty() {
                format!("No process {id} in this turn: no command has been left running.")
            } else {
                format!(
                    "No process {id} in this turn. Processes: {}.",
                    known.join(", ")
                )
            }
        })
    }

    fn entries(
        &self,
    ) -> std::sync::MutexGuard<'_, BTreeMap<String, Arc<tokio::sync::Mutex<Entry>>>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn running(&self) -> std::sync::MutexGuard<'_, usize> {
        self.running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// An answer that says the process ended has returned its last output.
fn returned(entry: &mut Entry, state: &ReportState) {
    if matches!(state, ReportState::Ended(_)) {
        entry.end_reported = true;
    }
}

/// Output a `status` read kept, followed by what this answer read.
fn take_pending(entry: &mut Entry, stdout: String, stderr: String) -> (String, String) {
    let mut out = std::mem::take(&mut entry.pending_stdout);
    out.push_str(&stdout);
    let mut err = std::mem::take(&mut entry.pending_stderr);
    err.push_str(&stderr);
    (out, err)
}

fn report(id: &str, entry: &Entry, stdout: String, stderr: String, state: ReportState) -> Report {
    Report {
        id: id.to_owned(),
        command: entry.command.clone(),
        elapsed: entry.started.elapsed(),
        timeout: entry.timeout,
        stdout,
        stderr,
        state,
        quiet_waits: entry.quiet_waits,
    }
}

#[cfg(test)]
#[path = "shell_processes_tests.rs"]
mod tests;
