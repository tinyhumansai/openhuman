//! Browser tasks run by TinyComputer's task members.
//!
//! A task is handed to the module with `StartTask` and followed with
//! `AwaitTask` until it finishes or pauses. The module drives its own browser
//! session, asks its decision model (Jev, OpenJev or Sage) for each step, and
//! hands a failed step to its rescue model before giving up. The host keeps
//! the policy: which surfaces and origins a task may touch, how many actions
//! it may take, and the approval for anything irreversible, which the module
//! surfaces as `needs_approval` and only `ContinueTask.approve` releases.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{de::DeserializeOwned, Serialize};
use tinycomputer_bus::agent::{
    names::methods, AgentResponse, AwaitTaskRequest, ContinueTaskRequest, PaymentMode,
    StartTaskRequest, SurfaceKind, TaskBudget, TaskConstraints, TaskId, TaskRef, TaskReport,
    TaskReportRequest, TaskView,
};

use crate::config::Config;

#[cfg(test)]
#[path = "browser_task_tests.rs"]
mod tests;

/// The longest one `AwaitTask` long-poll asks for. The module caps it at 60s.
const AWAIT_SLICE_MS: u64 = 30_000;
/// The bus deadline for one call; beyond the longest long-poll.
const CALL_TIMEOUT: Duration = Duration::from_secs(75);

/// What the browser tool asks for.
#[derive(Debug, Clone, Default)]
pub struct BrowserTask {
    /// The goal, in plain language.
    pub goal: String,
    /// Values the task may use, such as a name or an email address.
    pub facts: BTreeMap<String, String>,
    /// Origins the task's browser may load, in TinyComputer's
    /// `https://.host` spelling. Never empty: empty means any origin.
    pub origins: Vec<String>,
    /// Upper bound on the actions the task may take.
    pub max_actions: u32,
    /// A flow to run instead of having the module plan one from `goal`, such
    /// as a plan saved from an earlier run. `goal` then explains it.
    pub flow: Option<tinycomputer_bus::Flow>,
    /// The site the task starts on, from its starting address
    /// ([`super::site_memory_host::site_of`]): the module keeps what it learns
    /// under it, through the host's `SiteMemoryHost`, and hands the task what
    /// earlier tasks there learned.
    pub site: Option<String>,
}

/// Build the `StartTask` request for a browser-only task under host policy.
#[must_use]
pub fn start_request(config: &Config, task: &BrowserTask) -> StartTaskRequest {
    // With learning off the module is not told the site, so it neither reads
    // nor writes what earlier tasks there left.
    let site = task
        .site
        .clone()
        .filter(|_| super::site_memory_host::learning(config));
    StartTaskRequest {
        task: Some(task.goal.clone()),
        site,
        flow: task.flow.clone(),
        facts: task.facts.clone(),
        constraints: TaskConstraints {
            payment: PaymentMode::StopAtPayment,
            surfaces: vec![SurfaceKind::Browser],
            origins: task.origins.clone(),
            allow_destructive: false,
            browser_endpoint: None,
            headed: !config.browser.headless,
            // Read from the settings for every task, so a saved Chrome path
            // or profile applies without restarting the app.
            browser_executable: config.browser.chrome_path.clone(),
            browser_profile: (config.browser.profile_mode == "persistent")
                .then(|| config.browser.profile_path.clone())
                .flatten(),
        },
        budget: TaskBudget {
            max_actions: Some(task.max_actions),
            max_elapsed_ms: Some(config.browser.task_timeout_secs.saturating_mul(1000)),
            max_rescues: config.computer.max_rescues,
            ..TaskBudget::default()
        },
        trace: super::computer_config::tracing_enabled(config),
        ..StartTaskRequest::default()
    }
}

/// Start a browser task and follow it until it pauses, finishes, or the host
/// deadline passes (then the view still says `running`). The module hands the
/// task what earlier tasks on its site left, and keeps what it learns, through
/// the host's `SiteMemoryHost` (see [`super::site_memory_host`]).
///
/// # Errors
///
/// Returns a module, transport, or task error without page content.
pub async fn start(config: &Config, task: &BrowserTask) -> Result<TaskView, String> {
    if task.origins.is_empty() {
        return Err("a browser task needs at least one allowed origin".to_owned());
    }
    let mut request = start_request(config, task);
    let claimed = claim_profile(config, &mut request, super::profile_claim::TASK_WAIT_MS).await;
    tracing::debug!(
        origins = task.origins.len(),
        max_actions = task.max_actions,
        saved_flow = task.flow.is_some(),
        site_memory = request.site.is_some(),
        "[browser-task] starting"
    );
    let view: TaskView = match call(config, methods::START_TASK, request, true).await {
        Ok(view) => view,
        Err(error) => {
            // A task that did not start starts no browser on the profile.
            if let Some(profile) = claimed {
                super::profile_claim::release(config, &profile).await;
            }
            return Err(error);
        }
    };
    settle(config, view).await
}

/// Answer a paused task and follow it again.
///
/// # Errors
///
/// Returns a module, transport, or task error.
pub async fn resume(config: &Config, request: ContinueTaskRequest) -> Result<TaskView, String> {
    tracing::debug!(task = %request.id, approve = ?request.approve, "[browser-task] continuing");
    let view: TaskView = call(config, methods::CONTINUE_TASK, request, true).await?;
    settle(config, view).await
}

/// Keep following a task that was still running when the last call returned.
///
/// # Errors
///
/// Returns a module, transport, or task error.
pub async fn wait(config: &Config, id: TaskId) -> Result<TaskView, String> {
    let view: TaskView = call(
        config,
        methods::AWAIT_TASK,
        AwaitTaskRequest {
            id,
            timeout_ms: AWAIT_SLICE_MS,
        },
        false,
    )
    .await?;
    settle(config, view).await
}

/// Cancel a task.
///
/// # Errors
///
/// Returns a module or transport error.
pub async fn cancel(config: &Config, id: TaskId) -> Result<TaskView, String> {
    tracing::debug!(task = %id, "[browser-task] cancelling");
    call(config, methods::CANCEL_TASK, TaskRef { id }, false).await
}

/// The task's record: its steps, what it collected, and every rescue. Page
/// data can appear in it, so it is fetched confidentially and without the Jev
/// trace.
///
/// # Errors
///
/// Returns a module or transport error.
pub async fn report(config: &Config, id: TaskId) -> Result<TaskReport, String> {
    report_with(config, id, false).await
}

/// The task's record, with every Jev exchange when `trace` is set and the
/// task was started with `StartTask.trace`. A traced report can run to
/// megabytes, so only the tracing path asks for it.
///
/// # Errors
///
/// Returns a module or transport error.
pub(crate) async fn report_with(
    config: &Config,
    id: TaskId,
    trace: bool,
) -> Result<TaskReport, String> {
    call(
        config,
        methods::TASK_REPORT,
        TaskReportRequest { id, trace },
        true,
    )
    .await
}

/// Follow `view` until it stops or the host deadline passes, then record
/// how it stopped (see [`super::browser_task_report`]).
async fn settle(config: &Config, view: TaskView) -> Result<TaskView, String> {
    let view = follow(config, view).await?;
    super::browser_task_report::record(config, &view).await;
    Ok(view)
}

async fn follow(config: &Config, mut view: TaskView) -> Result<TaskView, String> {
    let deadline = Instant::now() + Duration::from_secs(config.browser.task_timeout_secs.max(1));
    while matches!(view.status, tinycomputer_bus::agent::TaskStatus::Running) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            tracing::debug!(task = %view.id, "[browser-task] host deadline reached while running");
            break;
        }
        let timeout_ms = u64::try_from(left.as_millis())
            .unwrap_or(u64::MAX)
            .min(AWAIT_SLICE_MS);
        view = call(
            config,
            methods::AWAIT_TASK,
            AwaitTaskRequest {
                id: view.id.clone(),
                timeout_ms,
            },
            false,
        )
        .await?;
    }
    tracing::debug!(task = %view.id, progress = view.progress, "[browser-task] paused or finished");
    Ok(view)
}

async fn call<Request: Serialize + Send, Reply: DeserializeOwned>(
    config: &Config,
    member: &str,
    request: Request,
    confidential: bool,
) -> Result<Reply, String> {
    let proxy = super::desktop::proxy(config)
        .await?
        .with_timeout(CALL_TIMEOUT);
    let response: AgentResponse<Reply> = if confidential {
        proxy.call_confidential(member, (request,)).await
    } else {
        proxy.call(member, (request,)).await
    }
    .map_err(|error| format!("browser task {member} failed: {error}"))?;
    unwrap_response(member, response)
}

/// Claims `request`'s persistent profile for the task's browser, or leaves it
/// a fresh profile when another browser still holds the folder after `wait`;
/// the folder it claimed. Chrome lets one browser at a time use a profile
/// folder, so a task waits a moment for one closing on it (the browser of the
/// task before it, which closes as it ends), and otherwise runs in a fresh
/// profile rather than fail to start its browser.
async fn claim_profile(
    config: &Config,
    request: &mut StartTaskRequest,
    wait_ms: u64,
) -> Option<String> {
    let profile = request.constraints.browser_profile.clone()?;
    match super::profile_claim::claim(config, &profile, wait_ms).await {
        super::profile_claim::Claim::Claimed(path) => Some(path),
        super::profile_claim::Claim::Fresh => {
            tracing::warn!(
                "[browser-task] another browser holds the persistent profile; this task runs in a fresh profile"
            );
            request.constraints.browser_profile = None;
            None
        }
    }
}

/// A module reply as its data, or as the error the tool reports: its code,
/// message and hint, and where the user fixes a Chrome the module was not
/// given or could not start ([`super::browser::chrome_hint`]).
fn unwrap_response<Reply>(member: &str, response: AgentResponse<Reply>) -> Result<Reply, String> {
    match (response.ok, response.data, response.error) {
        (true, Some(data), _) => Ok(data),
        (_, _, Some(error)) => {
            let host = super::browser::chrome_hint(&error.message).unwrap_or_default();
            let said = [error.message.as_str(), error.hint.as_str(), host]
                .into_iter()
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            Err(format!(
                "browser task {member} failed [{}]: {said}",
                error.code
            ))
        }
        _ => Err(format!("browser task {member} returned no result")),
    }
}
