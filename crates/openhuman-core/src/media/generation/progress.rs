//! Progress for long media-generation calls.
//!
//! An image takes up to five minutes and a video up to ten, and the turn waits
//! on the call the whole time. Two host-side pieces report while it runs,
//! through the harness's [`ToolRunContext::report_progress`] seam (which the
//! agent loop turns into `AgentEvent::ToolProgressDetail` for the call):
//!
//! - a **heartbeat** ([`with_progress_heartbeat`]) every
//!   [`HEARTBEAT_INTERVAL`] while the inner tool runs, naming the elapsed time;
//! - the **video job state** ([`video_status_observer`]), the
//!   `WaitPolicy::progress` hook the TinyInference poll loop calls on every
//!   poll. [`MediaArtifactTool`](super::artifact_tool::MediaArtifactTool)
//!   builds a fresh video tool for each call, bound to a [`JobStateSlot`] it
//!   owns, so the observer records into that call's slot and the next
//!   heartbeat reads it. Nothing is ambient: two concurrent calls each have
//!   their own slot.
//!
//! The host projects `ToolProgressDetail` onto the web channel as a
//! `tool_progress` event for the parent call
//! (`agent::tinyagents::observability`); the chat UI does not render it yet.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tinyagents_harness::tinyinference_video::{ProgressFn, VideoJobStatus};
use tinytools::{ToolProgress, ToolRunContext};

/// How often a running media call reports that it is still working.
pub(crate) const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

/// The latest provider job state seen by [`video_status_observer`] for one
/// media call. Owned by that call and shared with its heartbeat.
pub(crate) type JobStateSlot = Arc<Mutex<Option<String>>>;

/// The `WaitPolicy::progress` observer for one call: records each poll's job
/// state into `slot` for that call's heartbeat.
pub(crate) fn video_status_observer(slot: JobStateSlot) -> ProgressFn {
    Arc::new(move |status: &VideoJobStatus| {
        let state = status.state.to_string();
        *slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(state.clone());
        tracing::trace!(
            job_state = %state,
            "[media_generation] video job polled"
        );
    })
}

/// The heartbeat line for a call `elapsed` into its run.
pub(crate) fn heartbeat_message(label: &str, elapsed: Duration, job_state: Option<&str>) -> String {
    let secs = elapsed.as_secs();
    let elapsed = if secs >= 60 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    };
    match job_state {
        Some(state) => format!("{label} — {elapsed} elapsed (job {state})"),
        None => format!("{label} — {elapsed} elapsed"),
    }
}

/// Run `fut`, reporting a heartbeat through `context` every `interval` until
/// it finishes, naming the job state recorded in `state`. Without a context
/// there is nowhere to report, so `fut` runs as is.
pub(crate) async fn with_progress_heartbeat<F: std::future::Future>(
    context: Option<&dyn ToolRunContext>,
    label: &str,
    interval: Duration,
    state: JobStateSlot,
    fut: F,
) -> F::Output {
    let Some(context) = context else {
        return fut.await;
    };
    let started = tokio::time::Instant::now();
    let work = fut;
    tokio::pin!(work);
    let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
    loop {
        tokio::select! {
            output = &mut work => return output,
            _ = ticker.tick() => {
                let job_state = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                let message = heartbeat_message(label, started.elapsed(), job_state.as_deref());
                tracing::debug!(%message, "[media_generation] reporting progress");
                context.report_progress(ToolProgress::message(message));
            }
        }
    }
}

#[cfg(test)]
#[path = "progress_tests.rs"]
mod tests;
