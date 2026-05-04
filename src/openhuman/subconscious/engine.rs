//! Subconscious engine — SQLite-backed task evaluation and execution loop.
//!
//! On each tick: load due tasks from SQLite → log as in_progress →
//! evaluate with local model → execute "act" tasks → create escalations
//! for ambiguous tasks → update log entries in place.
//!
//! Overlap guard: each tick gets a generation counter. If a new tick starts
//! while the old one is in-flight, the old tick's in_progress entries are
//! marked as cancelled and its results are discarded.

use super::executor;
use super::prompt;
use super::situation_report::build_situation_report;
use super::store;
use super::types::{
    EscalationPriority, EvaluationResponse, SubconsciousStatus, SubconsciousTask, TaskEvaluation,
    TaskRecurrence, TaskSource, TickDecision, TickResult,
};
use crate::openhuman::memory::MemoryClientRef;
use anyhow::Result;
use executor::ExecutionOutcome;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

pub struct SubconsciousEngine {
    workspace_dir: PathBuf,
    interval_minutes: u32,
    context_budget_tokens: u32,
    enabled: bool,
    memory: Option<MemoryClientRef>,
    state: Mutex<EngineState>,
    /// Monotonically increasing tick generation. A tick checks this before
    /// writing results — if it has been bumped, the tick was superseded.
    tick_generation: AtomicU64,
}

struct EngineState {
    last_tick_at: f64,
    total_ticks: u64,
    consecutive_failures: u64,
    seeded: bool,
}

impl SubconsciousEngine {
    pub fn new(config: &crate::openhuman::config::Config, memory: Option<MemoryClientRef>) -> Self {
        Self::from_heartbeat_config(&config.heartbeat, config.workspace_dir.clone(), memory)
    }

    pub fn from_heartbeat_config(
        heartbeat: &crate::openhuman::config::HeartbeatConfig,
        workspace_dir: PathBuf,
        memory: Option<MemoryClientRef>,
    ) -> Self {
        // Seed default system tasks eagerly so they show in the UI immediately,
        // without waiting for the first tick.
        let seeded = match store::with_connection(&workspace_dir, store::seed_default_tasks) {
            Ok(count) => {
                if count > 0 {
                    info!("[subconscious] seeded {count} tasks on init");
                }
                true
            }
            Err(e) => {
                warn!("[subconscious] seed on init failed: {e}");
                false
            }
        };

        Self {
            workspace_dir,
            interval_minutes: heartbeat.interval_minutes.max(5),
            context_budget_tokens: heartbeat.context_budget_tokens,
            enabled: heartbeat.enabled && heartbeat.inference_enabled,
            memory,
            state: Mutex::new(EngineState {
                last_tick_at: 0.0,
                total_ticks: 0,
                consecutive_failures: 0,
                seeded,
            }),
            tick_generation: AtomicU64::new(0),
        }
    }

    /// Start the subconscious loop (runs until cancelled).
    ///
    /// Uses `sleep` after each tick (not `interval`) so ticks never stack up.
    /// If a tick takes longer than the interval, the next tick starts immediately
    /// after the previous one finishes — no overlap.
    pub async fn run(&self) -> Result<()> {
        if !self.enabled {
            info!("[subconscious] disabled, exiting");
            return Ok(());
        }

        let interval_secs = u64::from(self.interval_minutes) * 60;
        info!(
            "[subconscious] started: every {} minutes, budget {} tokens",
            self.interval_minutes, self.context_budget_tokens
        );

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;
            match self.tick().await {
                Ok(result) => {
                    info!(
                        "[subconscious] tick: executed={} escalated={} duration={}ms",
                        result.executed, result.escalated, result.duration_ms
                    );
                }
                Err(e) => {
                    warn!("[subconscious] tick error: {e}");
                }
            }
        }
    }

    /// Execute a single tick. Public for manual triggering via RPC.
    pub async fn tick(&self) -> Result<TickResult> {
        let started = std::time::Instant::now();
        let tick_at = now_secs();

        // Bump generation — any in-flight tick with an older generation is stale.
        let my_generation = self.tick_generation.fetch_add(1, Ordering::SeqCst) + 1;

        let mut state = self.state.lock().await;

        // Seed default tasks on first tick (fallback if init seeding failed)
        if !state.seeded {
            self.seed_tasks();
            state.seeded = true;
        }

        // Cancel any stale in_progress log entries from previous ticks
        let _ = store::with_connection(&self.workspace_dir, |conn| {
            let cancelled = store::cancel_stale_in_progress(conn)?;
            if cancelled > 0 {
                info!("[subconscious] cancelled {cancelled} stale in_progress entries");
            }
            Ok(())
        });

        let last_tick_at = state.last_tick_at;

        // 1. Load due tasks from SQLite
        let due_tasks =
            store::with_connection(&self.workspace_dir, |conn| store::due_tasks(conn, tick_at))?;

        if due_tasks.is_empty() {
            debug!("[subconscious] no due tasks");
            state.last_tick_at = tick_at;
            state.total_ticks += 1;
            return Ok(TickResult {
                tick_at,
                evaluations: vec![],
                executed: 0,
                escalated: 0,
                duration_ms: started.elapsed().as_millis() as u64,
            });
        }

        debug!("[subconscious] {} due tasks", due_tasks.len());

        // 2. Insert in_progress log entries for each due task
        let log_ids: HashMap<String, String> =
            store::with_connection(&self.workspace_dir, |conn| {
                let mut ids = HashMap::new();
                for task in &due_tasks {
                    match store::add_log_entry(
                        conn,
                        &task.id,
                        tick_at,
                        "in_progress",
                        Some("Evaluating..."),
                        None,
                    ) {
                        Ok(entry) => {
                            ids.insert(task.id.clone(), entry.id);
                        }
                        Err(e) => {
                            warn!(
                                "[subconscious] failed to log in_progress for '{}': {e}",
                                task.title
                            );
                        }
                    }
                }
                Ok(ids)
            })?;

        // 3. Build situation report
        let memory_ref = self.memory.as_ref().map(|m| m.as_ref());
        let report = build_situation_report(
            memory_ref,
            &self.workspace_dir,
            last_tick_at,
            self.context_budget_tokens,
        )
        .await;

        // 4. Load identity context
        let identity = prompt::load_identity_context(&self.workspace_dir);

        // Release lock during LLM calls
        drop(state);

        // 5. Evaluate tasks with local model
        let evaluations = self.evaluate_tasks(&due_tasks, &report, &identity).await;

        // Check if we were superseded by a newer tick
        if self.tick_generation.load(Ordering::SeqCst) != my_generation {
            info!("[subconscious] tick superseded by newer tick, discarding results");
            // Cancel our in_progress entries
            let _ = store::with_connection(&self.workspace_dir, |conn| {
                store::cancel_stale_in_progress(conn)
            });
            // Don't advance last_tick_at — next tick should re-fetch from
            // the same point so nothing is missed.
            let mut state = self.state.lock().await;
            state.total_ticks += 1;
            return Ok(TickResult {
                tick_at,
                evaluations: vec![],
                executed: 0,
                escalated: 0,
                duration_ms: started.elapsed().as_millis() as u64,
            });
        }

        // 6. Check if the evaluation itself failed (all tasks defaulted to noop
        //    due to LLM error). Individual task execution failures are tracked
        //    per-task and don't block the tick from advancing.
        let evaluation_failed = evaluations.iter().all(|e| {
            e.decision == TickDecision::Noop && e.reason.starts_with("Evaluation failed:")
        }) && !evaluations.is_empty();

        // 7. Execute based on decisions, updating log entries in place
        let mut executed = 0;
        let mut escalated = 0;

        for eval in &evaluations {
            let task = match due_tasks.iter().find(|t| t.id == eval.task_id) {
                Some(t) => t,
                None => continue,
            };
            let log_id = log_ids.get(&task.id).map(|s| s.as_str());

            match eval.decision {
                TickDecision::Act => {
                    self.handle_act(task, &report, &identity, tick_at, eval, log_id)
                        .await;
                    executed += 1;
                }
                TickDecision::Escalate => {
                    self.handle_escalate(task, tick_at, eval, log_id).await;
                    escalated += 1;
                }
                TickDecision::Noop => {
                    self.handle_noop(task, tick_at, eval, log_id).await;
                    self.advance_task_schedule(task, tick_at);
                }
            }
        }

        // 8. Mark any tasks that didn't get an evaluation as noop.
        //    This happens when the LLM returns results for only a subset of tasks.
        let evaluated_task_ids: std::collections::HashSet<&str> =
            evaluations.iter().map(|e| e.task_id.as_str()).collect();
        for task in &due_tasks {
            if !evaluated_task_ids.contains(task.id.as_str()) {
                if let Some(lid) = log_ids.get(&task.id) {
                    let _ = store::with_connection(&self.workspace_dir, |conn| {
                        store::update_log_entry(
                            conn,
                            lid,
                            "noop",
                            Some("No evaluation returned by model"),
                            None,
                        )
                    });
                }
            }
        }

        // 9. Update state
        let mut state = self.state.lock().await;
        state.total_ticks += 1;
        if evaluation_failed {
            state.consecutive_failures += 1;
            // Don't advance last_tick_at — the LLM couldn't evaluate anything,
            // so the next tick should re-fetch from the same point.
        } else {
            state.consecutive_failures = 0;
            state.last_tick_at = tick_at;
        }

        Ok(TickResult {
            tick_at,
            evaluations,
            executed,
            escalated,
            duration_ms: started.elapsed().as_millis() as u64,
        })
    }

    /// Get current status.
    pub async fn status(&self) -> SubconsciousStatus {
        let state = self.state.lock().await;
        let (task_count, pending_escalations) =
            store::with_connection(&self.workspace_dir, |conn| {
                Ok((
                    store::task_count(conn).unwrap_or(0),
                    store::pending_escalation_count(conn).unwrap_or(0),
                ))
            })
            .unwrap_or((0, 0));

        SubconsciousStatus {
            enabled: self.enabled,
            interval_minutes: self.interval_minutes,
            last_tick_at: if state.last_tick_at > 0.0 {
                Some(state.last_tick_at)
            } else {
                None
            },
            total_ticks: state.total_ticks,
            task_count,
            pending_escalations,
            consecutive_failures: state.consecutive_failures,
        }
    }

    /// Add a new task. All tasks are evaluated on every tick — no scheduling needed.
    pub async fn add_task(&self, title: &str, source: TaskSource) -> Result<SubconsciousTask> {
        let task = store::with_connection(&self.workspace_dir, |conn| {
            store::add_task(conn, title, source, TaskRecurrence::Pending)
        })?;
        info!("[subconscious] added task: {}", title);
        Ok(task)
    }

    /// Approve an escalation — execute the task then mark approved.
    pub async fn approve_escalation(&self, escalation_id: &str) -> Result<()> {
        let (escalation, task) = store::with_connection(&self.workspace_dir, |conn| {
            let esc = store::get_escalation(conn, escalation_id)?;
            let task = store::get_task(conn, &esc.task_id)?;
            Ok((esc, task))
        })?;

        info!(
            "[subconscious] approved escalation '{}' for task '{}'",
            escalation.title, task.title
        );

        // Execute the task
        let identity = prompt::load_identity_context(&self.workspace_dir);
        let memory_ref = self.memory.as_ref().map(|m| m.as_ref());
        let report = build_situation_report(
            memory_ref,
            &self.workspace_dir,
            0.0, // fresh report for execution
            self.context_budget_tokens,
        )
        .await;

        let tick_at = now_secs();
        let result = executor::execute_approved_write(&task, &report, &identity).await;
        let (result_text, duration) = match &result {
            Ok(r) => (r.output.clone(), Some(r.duration_ms as i64)),
            Err(e) => (format!("Execution failed: {e}"), None),
        };

        store::with_connection(&self.workspace_dir, |conn| {
            store::add_log_entry(conn, &task.id, tick_at, "act", Some(&result_text), duration)?;
            store::resolve_escalation(
                conn,
                escalation_id,
                &super::types::EscalationStatus::Approved,
            )?;
            if task.recurrence == TaskRecurrence::Once {
                store::mark_task_completed(conn, &task.id)?;
            } else {
                self.advance_task_schedule_in_conn(conn, &task, tick_at);
            }
            Ok(())
        })?;

        Ok(())
    }

    /// Dismiss an escalation — log and don't execute.
    pub async fn dismiss_escalation(&self, escalation_id: &str) -> Result<()> {
        store::with_connection(&self.workspace_dir, |conn| {
            let esc = store::get_escalation(conn, escalation_id)?;
            store::add_log_entry(
                conn,
                &esc.task_id,
                now_secs(),
                "dismissed",
                Some("Dismissed by user"),
                None,
            )?;
            store::resolve_escalation(
                conn,
                escalation_id,
                &super::types::EscalationStatus::Dismissed,
            )?;
            Ok(())
        })
    }

    // ── Internal methods ─────────────────────────────────────────────────────

    fn seed_tasks(&self) {
        match store::with_connection(&self.workspace_dir, store::seed_default_tasks) {
            Ok(count) => {
                if count > 0 {
                    info!("[subconscious] seeded {count} default tasks");
                }
            }
            Err(e) => warn!("[subconscious] seed failed: {e}"),
        }
    }

    async fn evaluate_tasks(
        &self,
        tasks: &[SubconsciousTask],
        report: &str,
        identity: &str,
    ) -> Vec<TaskEvaluation> {
        let prompt_text = prompt::build_evaluation_prompt(tasks, report, identity);

        let config = match crate::openhuman::config::Config::load_or_init().await {
            Ok(c) => c,
            Err(e) => {
                warn!("[subconscious] config load failed: {e}");
                return tasks
                    .iter()
                    .map(|t| TaskEvaluation {
                        task_id: t.id.clone(),
                        decision: TickDecision::Noop,
                        reason: format!("Config load failed: {e}"),
                    })
                    .collect();
            }
        };

        let messages = vec![
            crate::openhuman::local_ai::ops::LocalAiChatMessage {
                role: "system".to_string(),
                content: prompt_text,
            },
            crate::openhuman::local_ai::ops::LocalAiChatMessage {
                role: "user".to_string(),
                content: "Evaluate the due tasks. Reply with JSON only.".to_string(),
            },
        ];

        match crate::openhuman::local_ai::ops::local_ai_chat(&config, messages, None).await {
            Ok(outcome) => parse_evaluations(&outcome.value, tasks),
            Err(e) => {
                warn!("[subconscious] evaluation failed: {e}");
                tasks
                    .iter()
                    .map(|t| TaskEvaluation {
                        task_id: t.id.clone(),
                        decision: TickDecision::Noop,
                        reason: format!("Evaluation failed: {e}"),
                    })
                    .collect()
            }
        }
    }

    /// Handle an "act" decision. Individual execution failures are logged
    /// per-task but don't block the tick from advancing.
    async fn handle_act(
        &self,
        task: &SubconsciousTask,
        report: &str,
        identity: &str,
        tick_at: f64,
        eval: &TaskEvaluation,
        log_id: Option<&str>,
    ) {
        info!(
            "[subconscious] executing task '{}': {}",
            task.title, eval.reason
        );

        let result = executor::execute_task(task, report, identity).await;

        match &result {
            Ok(ExecutionOutcome::Completed(r)) => {
                let _ = store::with_connection(&self.workspace_dir, |conn| {
                    let duration = Some(r.duration_ms as i64);
                    if let Some(lid) = log_id {
                        store::update_log_entry(conn, lid, "act", Some(&r.output), duration)?;
                    } else {
                        store::add_log_entry(
                            conn,
                            &task.id,
                            tick_at,
                            "act",
                            Some(&r.output),
                            duration,
                        )?;
                    }
                    if task.recurrence == TaskRecurrence::Once {
                        store::mark_task_completed(conn, &task.id)?;
                        info!("[subconscious] one-off task '{}' completed", task.title);
                    } else {
                        self.advance_task_schedule_in_conn(conn, task, tick_at);
                    }
                    Ok(())
                });
            }
            Ok(ExecutionOutcome::UnapprovedWrite {
                recommendation,
                duration_ms,
            }) => {
                // agentic-v1 wants to take a write action the user didn't ask for.
                // Create an escalation so the user can approve or dismiss.
                info!(
                    "[subconscious] unapproved write for '{}': {}",
                    task.title, recommendation
                );
                let _ = store::with_connection(&self.workspace_dir, |conn| {
                    let duration = Some(*duration_ms as i64);
                    let effective_log_id = if let Some(lid) = log_id {
                        store::update_log_entry(
                            conn,
                            lid,
                            "escalate",
                            Some(recommendation),
                            duration,
                        )?;
                        lid.to_string()
                    } else {
                        let entry = store::add_log_entry(
                            conn,
                            &task.id,
                            tick_at,
                            "escalate",
                            Some(recommendation),
                            duration,
                        )?;
                        entry.id
                    };
                    store::add_escalation(
                        conn,
                        &task.id,
                        Some(&effective_log_id),
                        &task.title,
                        recommendation,
                        &EscalationPriority::Important,
                    )?;
                    Ok(())
                });
            }
            Err(e) => {
                let msg = format!("Execution failed: {e}");
                let _ = store::with_connection(&self.workspace_dir, |conn| {
                    if let Some(lid) = log_id {
                        store::update_log_entry(conn, lid, "failed", Some(&msg), None)?;
                    } else {
                        store::add_log_entry(conn, &task.id, tick_at, "failed", Some(&msg), None)?;
                    }
                    Ok(())
                });
            }
        }
    }

    async fn handle_escalate(
        &self,
        task: &SubconsciousTask,
        tick_at: f64,
        eval: &TaskEvaluation,
        log_id: Option<&str>,
    ) {
        info!(
            "[subconscious] escalating task '{}': {}",
            task.title, eval.reason
        );

        let _ = store::with_connection(&self.workspace_dir, |conn| {
            let effective_log_id = if let Some(lid) = log_id {
                store::update_log_entry(conn, lid, "escalate", Some(&eval.reason), None)?;
                lid.to_string()
            } else {
                let entry = store::add_log_entry(
                    conn,
                    &task.id,
                    tick_at,
                    "escalate",
                    Some(&eval.reason),
                    None,
                )?;
                entry.id
            };
            store::add_escalation(
                conn,
                &task.id,
                Some(&effective_log_id),
                &task.title,
                &eval.reason,
                &EscalationPriority::Important,
            )?;
            Ok(())
        });
    }

    async fn handle_noop(
        &self,
        task: &SubconsciousTask,
        tick_at: f64,
        eval: &TaskEvaluation,
        log_id: Option<&str>,
    ) {
        debug!("[subconscious] noop for '{}': {}", task.title, eval.reason);
        let _ = store::with_connection(&self.workspace_dir, |conn| {
            if let Some(lid) = log_id {
                store::update_log_entry(conn, lid, "noop", Some(&eval.reason), None)?;
            } else {
                store::add_log_entry(conn, &task.id, tick_at, "noop", Some(&eval.reason), None)?;
            }
            Ok(())
        });
    }

    fn advance_task_schedule(&self, task: &SubconsciousTask, tick_at: f64) {
        let _ = store::with_connection(&self.workspace_dir, |conn| {
            self.advance_task_schedule_in_conn(conn, task, tick_at);
            Ok(())
        });
    }

    fn advance_task_schedule_in_conn(
        &self,
        conn: &rusqlite::Connection,
        task: &SubconsciousTask,
        tick_at: f64,
    ) {
        if let TaskRecurrence::Cron(ref expr) = task.recurrence {
            let next = store::compute_next_run(expr);
            let _ = store::update_task_run_times(conn, &task.id, tick_at, next);
        } else if task.recurrence == TaskRecurrence::Pending {
            // Pending tasks run on every tick until classified
            let next = tick_at + (f64::from(self.interval_minutes) * 60.0);
            let _ = store::update_task_run_times(conn, &task.id, tick_at, Some(next));
        }
    }
}

/// Parse the local model's evaluation response into per-task decisions.
fn parse_evaluations(text: &str, tasks: &[SubconsciousTask]) -> Vec<TaskEvaluation> {
    let json_text = extract_json(text);

    // Try parsing as EvaluationResponse
    if let Ok(response) = serde_json::from_str::<EvaluationResponse>(json_text) {
        if !response.evaluations.is_empty() {
            return response.evaluations;
        }
    }

    // Try parsing as a bare array of evaluations
    if let Ok(evals) = serde_json::from_str::<Vec<TaskEvaluation>>(json_text) {
        if !evals.is_empty() {
            return evals;
        }
    }

    warn!("[subconscious] could not parse evaluation response, defaulting all to noop");
    tasks
        .iter()
        .map(|t| TaskEvaluation {
            task_id: t.id.clone(),
            decision: TickDecision::Noop,
            reason: "Unparseable evaluation response".to_string(),
        })
        .collect()
}

fn extract_json(text: &str) -> &str {
    let trimmed = text.trim();
    let obj_start = trimmed.find('{');
    let arr_start = trimmed.find('[');
    let start = match (obj_start, arr_start) {
        (Some(o), Some(a)) => o.min(a),
        (Some(o), None) => o,
        (None, Some(a)) => a,
        (None, None) => return trimmed,
    };
    let end = if trimmed.as_bytes().get(start) == Some(&b'[') {
        trimmed.rfind(']').map(|i| i + 1)
    } else {
        trimmed.rfind('}').map(|i| i + 1)
    };
    let end = end.unwrap_or(trimmed.len());
    if start < end {
        &trimmed[start..end]
    } else {
        trimmed
    }
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
