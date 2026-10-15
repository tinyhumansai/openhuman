//! Agent-facing browser backed by the TinyComputer module's browser and task members.
#[path = "browser_lifecycle.rs"]
mod cleanup;
#[path = "browser_pending.rs"]
mod pending;
#[path = "browser_session_pool.rs"]
mod session_pool;
#[path = "browser_task_actions.rs"]
mod task_actions;
#[path = "browser_unattended.rs"]
mod unattended;
use crate::agent::turn_origin::AgentTurnOrigin;
use crate::modules::browser::BrowserClient;
use crate::security::approval::{ApprovalGate, GateOutcome};
use crate::security::SecurityPolicy;
use async_trait::async_trait;
use pending::{approval_target, needs_host_confirmation, Pending};
use serde_json::{json, Value};
use session_pool::{
    browser_session_fingerprint, evict_thread_sessions, requires_rebind, thread_sessions,
    ThreadSession,
};
#[cfg(test)]
use session_pool::{MAX_THREAD_SESSIONS, SESSION_IDLE_TTL};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::{collections::HashMap, time::Duration};
use std::{
    sync::{Arc, Mutex as StdMutex},
    time::Instant,
};
use task_actions::{
    approve_task_action, direct_action, host_hint, required, task_inputs, task_outcome,
};
use tinycomputer_bus::agent::{ContinueTaskRequest, TaskId, TaskStatus, TaskView};
use tinycomputer_bus::browser::{
    Action, DownloadState, DownloadWaitRequest, NavigateRequest, ReadRequest, SessionId,
    SessionOptions, SnapshotRequest,
};
use tinytools::{Tool, ToolCallOptions, ToolResult, ToolRunContext};
use tokio::sync::Mutex;

async fn approve_browser_action(
    client: &BrowserClient,
    session: &SessionId,
    action: &Action,
    force: bool,
    origin: Option<&AgentTurnOrigin>,
) -> anyhow::Result<()> {
    if !force && !needs_host_confirmation(action) {
        return Ok(());
    }
    let action_json = serde_json::to_value(action)?;
    let kind = action_json["action"].as_str().unwrap_or("action");
    // Nobody waits on an unattended approval, so the page needs no binding.
    let action_digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&action_json)?));
    if unattended::allow(origin, &client.config().browser, kind, &action_digest) {
        return Ok(());
    }
    let gate = ApprovalGate::try_global().ok_or_else(|| {
        anyhow::anyhow!("[policy-denied] Browser action needs an interactive host approval gate")
    })?;
    let before = client
        .read_page(
            session,
            ReadRequest {
                max_chars: 1,
                ..ReadRequest::default()
            },
        )
        .await?;
    let origin = reqwest::Url::parse(&before.url)
        .ok()
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|| "unknown page".into());
    let digest = Sha256::digest(serde_json::to_vec(
        &json!({"url": before.url, "action": action_json}),
    )?);
    let (target_ref, target_detail) = approval_target(action);
    let input_detail = match action {
        Action::Fill { value, .. } => format!(" ({} input characters)", value.chars().count()),
        Action::Type { text, .. } => format!(" ({} input characters)", text.chars().count()),
        Action::Press { key, .. } => format!(" ({key})"),
        _ => String::new(),
    };
    let digest_hex = format!("{digest:x}");
    let display_target = format!(
        "{kind}{target_detail}{input_detail} on {origin} [action {}] — review the browser tool input before allowing",
        &digest_hex[..12]
    );
    let summary = format!("Browser {display_target}");
    // Bind action and URL with a digest, and show a bounded selector preview.
    let args = json!({"action": kind, "origin": origin, "target": display_target,
        "target_ref": target_ref, "exact_action_sha256": digest_hex});
    match gate.intercept_forced("browser", &summary, args).await {
        GateOutcome::Allow => {}
        GateOutcome::Deny { reason } => anyhow::bail!("{reason}"),
    }
    let after = client
        .read_page(
            session,
            ReadRequest {
                max_chars: 1,
                ..ReadRequest::default()
            },
        )
        .await?;
    if after.url != before.url {
        anyhow::bail!("Browser page changed during host approval");
    }
    Ok(())
}

pub struct BrowserTool {
    security: Arc<SecurityPolicy>,
    client: Arc<BrowserClient>,
    session: Mutex<Option<SessionId>>,
    bound_origin: Mutex<Option<String>>,
    pending: Mutex<Option<Pending>>,
    thread_key: StdMutex<Option<String>>,
    max_steps: usize,
}

impl BrowserTool {
    pub fn new(
        security: Arc<SecurityPolicy>,
        client: Arc<BrowserClient>,
        max_steps: usize,
    ) -> Self {
        Self {
            security,
            client,
            session: Mutex::new(None),
            bound_origin: Mutex::new(None),
            pending: Mutex::new(None),
            thread_key: StdMutex::new(None),
            max_steps: max_steps.clamp(1, 100),
        }
    }

    async fn session(&self) -> anyhow::Result<SessionId> {
        self.session_for_url(None).await
    }

    async fn session_for_url(&self, url: Option<&str>) -> anyhow::Result<SessionId> {
        let requested_origin = url
            .map(|url| self.client.explicit_origin(url))
            .transpose()?
            .flatten();
        let mut held = self.session.lock().await;
        let mut bound = self.bound_origin.lock().await;
        let thread_key = self.thread_key.lock().ok().and_then(|key| key.clone());
        if let Some(key) = thread_key {
            let mut sessions = thread_sessions().lock().await;
            let now = Instant::now();
            let config_fingerprint = browser_session_fingerprint(&self.client);
            let expired = evict_thread_sessions(&mut sessions, now, false);
            for value in expired {
                let _ = value.client.close_session(&value.id).await;
            }
            if sessions.get(&key).is_some_and(|value| {
                value.config_fingerprint != config_fingerprint
                    || requires_rebind(value.bound_origin.as_deref(), requested_origin.as_deref())
            }) {
                let stale = sessions.get(&key).expect("entry checked above");
                *held = None;
                *bound = None;
                *self.pending.lock().await = None;
                // Keep its entry until close succeeds; retry before replacement.
                stale.client.close_session(&stale.id).await?;
                sessions.remove(&key);
            }
            if let Some(value) = sessions.get_mut(&key) {
                value.last_used = now;
                *held = Some(value.id.clone());
                *bound = value.bound_origin.clone();
                return Ok(value.id.clone());
            }
            // The shared entry may have been evicted while this tool kept its
            // local handle. Never return that closed session to a later turn.
            *held = None;
            *bound = None;
            let capacity = evict_thread_sessions(&mut sessions, now, true);
            for value in capacity {
                let _ = value.client.close_session(&value.id).await;
            }
            let id = self.open_scoped_session(url).await?;
            sessions.insert(
                key,
                ThreadSession {
                    id: id.clone(),
                    client: self.client.clone(),
                    last_used: now,
                    config_fingerprint,
                    bound_origin: requested_origin.clone(),
                },
            );
            *held = Some(id.clone());
            *bound = requested_origin;
            return Ok(id);
        }
        if let Some(id) = held.as_ref() {
            if !requires_rebind(bound.as_deref(), requested_origin.as_deref()) {
                return Ok(id.clone());
            }
            self.client.close_session(id).await?;
            *held = None;
            *bound = None;
            *self.pending.lock().await = None;
        }
        let id = self.open_scoped_session(url).await?;
        *held = Some(id.clone());
        *bound = requested_origin;
        Ok(id)
    }

    async fn open_scoped_session(&self, url: Option<&str>) -> anyhow::Result<SessionId> {
        Ok(match url {
            Some(url) => self.client.open_session_for_url(url).await?.id,
            None => {
                self.client
                    .open_session(SessionOptions::default())
                    .await?
                    .id
            }
        })
    }

    async fn close(&self) -> anyhow::Result<Value> {
        let thread_key = self.thread_key.lock().ok().and_then(|key| key.clone());
        let mut held = self.session.lock().await;
        *self.bound_origin.lock().await = None;
        *self.pending.lock().await = None;
        let id = if let Some(key) = thread_key {
            let mut sessions = thread_sessions().lock().await;
            // A stale tool must not remove or close a replacement session
            // opened by another tool for the same conversation.
            let owns_entry = sessions
                .get(&key)
                .is_some_and(|entry| held.as_ref().is_none_or(|id| entry.id == *id));
            let removed = owns_entry.then(|| sessions.remove(&key)).flatten();
            held.take()
                .filter(|id| removed.as_ref().is_some_and(|entry| entry.id == *id))
                .or_else(|| removed.map(|entry| entry.id))
        } else {
            held.take()
        };
        drop(held);
        if let Some(id) = id {
            self.client.close_session(&id).await?;
        }
        Ok(json!({"closed": true}))
    }

    async fn task(&self, args: &Value) -> anyhow::Result<Value> {
        let mut goal = required(args, "goal")?.to_owned();
        let site = args["url"]
            .as_str()
            .and_then(crate::modules::site_memory_host::site_of);
        let origins = match args["url"].as_str().filter(|url| !url.trim().is_empty()) {
            Some(url) => {
                self.client.check_url(url)?;
                goal = format!("Start at {url}. {goal}");
                self.client
                    .explicit_origin(url)?
                    .map_or_else(|| self.client.task_origins(), |origin| vec![origin])
            }
            None => self.client.task_origins(),
        };
        let facts = task_inputs(args)?;
        let flow = match args.get("flow").filter(|flow| !flow.is_null()) {
            Some(flow) => Some(
                serde_json::from_value::<tinycomputer_bus::Flow>(flow.clone())
                    .map_err(|error| anyhow::anyhow!("Invalid flow: {error}"))?,
            ),
            None => None,
        };
        let task = crate::modules::browser_task::BrowserTask {
            goal,
            facts,
            origins,
            max_actions: u32::try_from(self.max_steps).unwrap_or(u32::MAX),
            flow,
            site,
        };
        // The browser this conversation keeps open holds the persistent
        // profile, which the task's own browser needs: it is let go first.
        if self.client.config().browser.profile_mode == "persistent" {
            if let Err(error) = self.close().await {
                tracing::debug!(
                    error = %error,
                    "[tool.browser] could not close this conversation's browser before a task"
                );
            }
        }
        let view = crate::modules::browser_task::start(self.client.config(), &task)
            .await
            .map_err(anyhow::Error::msg)?;
        self.report(view).await
    }

    /// Answer a paused task (inputs, a free-text answer) or keep following a
    /// running one.
    async fn task_continue(&self, args: &Value) -> anyhow::Result<Value> {
        let id = TaskId::new(required(args, "task_id")?);
        let inputs = task_inputs(args)?;
        let answer = args["answer"].as_str().map(str::to_owned);
        let view = if inputs.is_empty() && answer.is_none() {
            crate::modules::browser_task::wait(self.client.config(), id).await
        } else {
            crate::modules::browser_task::resume(
                self.client.config(),
                ContinueTaskRequest {
                    id,
                    inputs,
                    answer,
                    ..ContinueTaskRequest::default()
                },
            )
            .await
        }
        .map_err(anyhow::Error::msg)?;
        self.report(view).await
    }

    async fn task_cancel(&self, args: &Value) -> anyhow::Result<Value> {
        let id = TaskId::new(required(args, "task_id")?);
        *self.pending.lock().await = None;
        let view = crate::modules::browser_task::cancel(self.client.config(), id)
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(serde_json::to_value(view)?)
    }

    /// Report a task view as the tool's outcome ([`task_outcome`]): a task
    /// that failed or was cancelled under the caller is an error.
    async fn report(&self, view: TaskView) -> anyhow::Result<Value> {
        let output = self.describe(&view).await?;
        task_outcome(&view, output)
    }

    /// Render a task view; a `needs_approval` pause is held with a one-use
    /// token that only `confirm_pending` (through the host gate) can spend.
    async fn describe(&self, view: &TaskView) -> anyhow::Result<Value> {
        let mut output = serde_json::to_value(view)?;
        if let Some(hint) = host_hint(view) {
            output["host_hint"] = json!(hint);
        }
        if let TaskStatus::NeedsApproval { action, target, .. } = &view.status {
            let token = uuid::Uuid::new_v4().to_string();
            *self.pending.lock().await = Some(Pending {
                task: view.id.clone(),
                action: action.clone(),
                target: target.clone(),
                token: token.clone(),
            });
            output["pending"] = json!({"task_id": view.id, "action": action, "target": target,
                "token": token, "approval": "Call confirm_pending with this token to request host approval for this exact action"});
        } else {
            *self.pending.lock().await = None;
        }
        Ok(output)
    }

    async fn confirm_pending(
        &self,
        args: &Value,
        origin: Option<&AgentTurnOrigin>,
    ) -> anyhow::Result<Value> {
        let mut slot = self.pending.lock().await;
        let held = slot
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No pending browser action"))?;
        if !held.matches(args) {
            anyhow::bail!("Pending browser action does not match the approved request");
        }
        let pending = slot.take().expect("pending checked above");
        drop(slot);
        let approved = approve_task_action(&pending, &self.client.config().browser, origin).await?;
        let view = crate::modules::browser_task::resume(
            self.client.config(),
            ContinueTaskRequest {
                id: pending.task,
                approve: Some(approved),
                ..ContinueTaskRequest::default()
            },
        )
        .await
        .map_err(anyhow::Error::msg)?;
        let mut output = self.describe(&view).await?;
        if !approved {
            output["approval"] = json!("denied by the host");
        }
        task_outcome(&view, output)
    }

    /// `origin` is the turn this call runs under; it decides only whether an
    /// allow-listed action may skip the forced approval gate.
    async fn run(&self, args: &Value, origin: Option<&AgentTurnOrigin>) -> anyhow::Result<Value> {
        let verb = required(args, "action")?;
        if verb == "close" {
            return self.close().await;
        }
        match verb {
            "confirm_pending" => return self.confirm_pending(args, origin).await,
            "task" => return self.task(args).await,
            "task_continue" => return self.task_continue(args).await,
            "task_cancel" => return self.task_cancel(args).await,
            _ => {}
        }
        let starting_url = match verb {
            "open" => Some(required(args, "url")?),
            _ => None,
        };
        let id = self.session_for_url(starting_url).await?;
        match verb {
            "open" => Ok(serde_json::to_value(
                self.client
                    .navigate(&id, NavigateRequest::new(required(args, "url")?))
                    .await?,
            )?),
            "snapshot" => Ok(serde_json::to_value(
                self.client
                    .snapshot(
                        &id,
                        SnapshotRequest {
                            interactive_only: args["interactive_only"].as_bool().unwrap_or(false),
                            compact: args["compact"].as_bool().unwrap_or(true),
                            depth: args["depth"].as_u64().and_then(|v| u32::try_from(v).ok()),
                            max_chars: 50_000,
                            ..SnapshotRequest::default()
                        },
                    )
                    .await?,
            )?),
            "read_page" => Ok(serde_json::to_value(
                self.client
                    .read_page(
                        &id,
                        ReadRequest {
                            max_chars: 50_000,
                            ..ReadRequest::default()
                        },
                    )
                    .await?,
            )?),
            "get_title" | "get_url" => {
                let p = self
                    .client
                    .read_page(
                        &id,
                        ReadRequest {
                            max_chars: 1,
                            ..ReadRequest::default()
                        },
                    )
                    .await?;
                Ok(if verb == "get_title" {
                    json!({"title":p.title})
                } else {
                    json!({"url":p.url})
                })
            }
            "list_downloads" => Ok(serde_json::to_value(
                self.client.list_downloads(&id).await?,
            )?),
            "wait_download" => {
                let d = self
                    .client
                    .wait_download(
                        &id,
                        DownloadWaitRequest {
                            timeout_ms: args["timeout_ms"].as_u64(),
                        },
                    )
                    .await?;
                if !matches!(d.state, DownloadState::Completed) {
                    anyhow::bail!("Download did not complete: {:?}", d.state);
                }
                let path = d
                    .path
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("No permitted download path"))?;
                let file = tokio::fs::metadata(path).await?;
                if !file.is_file() || file.len() == 0 || file.len() != d.received_bytes {
                    anyhow::bail!("Downloaded file bytes differ from tracked download");
                }
                Ok(serde_json::to_value(d)?)
            }
            _ => {
                let action = direct_action(args)?;
                approve_browser_action(&self.client, &id, &action, false, origin).await?;
                Ok(serde_json::to_value(
                    self.client.perform(&id, action).await?,
                )?)
            }
        }
    }
}

/// The browser tool's deadline: the approval gate's ten-minute park plus two
/// minutes for the action itself. See `BrowserTool::timeout_policy`.
pub(crate) const BROWSER_TOOL_TIMEOUT_MS: u64 = 12 * 60 * 1000;

#[async_trait]
impl Tool for BrowserTool {
    fn exposure(&self) -> tinytools::ToolExposure {
        tinytools::ToolExposure::Deferred
    }
    fn name(&self) -> &str {
        "browser"
    }
    fn description(&self) -> &str {
        concat!(
            "TinyComputer website operations. Call action=open with the starting URL in this tool ",
            "before snapshot or read_page. browser_open is a separate one-shot session. ",
            "Then use snapshot for current ",
            "accessibility refs such as @e1 and read_page for visible prose. Refs expire after ",
            "navigation or a new snapshot; take a fresh snapshot instead of guessing a stale ref. ",
            "For multi-step work, use task with an optional starting url, an observable final-state goal, and named exact ",
            "input values; TinyComputer runs it in its own browser session, and a failed step is ",
            "handed to its rescue model before the task fails. The reply's status says what it needs: ",
            "running (call task_continue with task_id to keep following it), needs_input (task_continue ",
            "with inputs), needs_human (ask the user, then task_continue with answer=done), needs_approval ",
            "(returns a pending token; use confirm_pending only through the host approval mechanism), ",
            "checkpoint (always before payment), done, or failed with a hint. task_cancel stops it. ",
            "Direct consequential clicks and key presses also require host approval. For downloads, inspect list_downloads and call wait_download; ",
            "success is reported only after the tracked file path and byte count are verified. ",
            "Keep the same conversation session across turns and close it when finished. Navigation ",
            "obeys the shared allowed websites list; do not try to evade a blocked destination."
        )
    }
    fn parameters_schema(&self) -> Value {
        tinycomputer_bus::browser::tool::parameters_schema()
    }
    /// Direct consequential actions park on the approval gate *inside*
    /// `execute` (`intercept_forced`), which waits up to the gate's TTL. The
    /// inherited 120s per-tool deadline would cut that park long before the
    /// user can answer, so the browser carries its own budget: the longest
    /// park plus time for the action itself.
    fn timeout_policy(&self, _args: &Value) -> tinytools::ToolTimeout {
        tinytools::ToolTimeout::Millis(BROWSER_TOOL_TIMEOUT_MS)
    }
    fn external_effect_with_args(&self, args: &Value) -> bool {
        // Gate direct mutations before perform; task steps pause for approval.
        // An outer effect would gate twice without covering task-selected steps.
        let _ = args;
        false
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        if !self.security.can_act() {
            return Ok(ToolResult::error(
                "[policy-blocked] Action blocked: autonomy is read-only",
            ));
        }
        // A task counts once here; its own steps are bounded by max_actions.
        if !self.security.record_action() {
            return Ok(ToolResult::error("Action blocked: rate limit exceeded"));
        }
        // The typed origin bound to this turn's immutable CoreContext by its
        // entry point (cron, background job, workflow, chat).
        let origin = crate::core::runtime::CoreContext::current_turn_origin();
        match self.run(&args, origin.as_ref()).await {
            Ok(v) => Ok(ToolResult::success(serde_json::to_string_pretty(&v)?)),
            Err(e) => Ok(ToolResult::error(e.to_string())),
        }
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        if let Some(thread_id) = context.and_then(ToolRunContext::thread_id) {
            let scope = crate::core::runtime::agent_scope_dir(self.client.config());
            let key = format!("{}:{thread_id}", scope.display());
            if let Ok(mut held) = self.thread_key.lock() {
                *held = Some(key);
            }
        }
        self.execute(args).await
    }
}

#[cfg(test)]
#[path = "browser_computer_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "browser_task_outcome_tests.rs"]
mod task_outcome_tests;
