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
use super::reflection::{apply_cap, hydrate_draft, Reflection, ReflectionDraft};
use super::reflection_store;
use super::situation_report::build_situation_report;
use super::source_chunk::resolve_chunks;
use super::store;
use super::types::{
    EscalationPriority, EvaluationResponse, SubconsciousStatus, SubconsciousTask, TaskEvaluation,
    TaskRecurrence, TaskSource, TickDecision, TickResult,
};
use crate::openhuman::memory::tree::chat::{
    build_chat_provider, ChatConsumer, ChatPrompt, ChatProvider,
};
use crate::openhuman::memory::MemoryClientRef;
use anyhow::Result;
use executor::ExecutionOutcome;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
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

        // Restore `last_tick_at` from `subconscious_state` so the
        // situation-report cutoff survives process restarts. Without
        // this every restart cold-starts the LLM, which sees the same
        // memory-tree rows again and re-emits near-duplicate reflections
        // (no insert-time dedupe in `persist_and_surface_reflections`).
        // 0.0 on first run / load failure mirrors the previous default.
        let last_tick_at = match store::with_connection(&workspace_dir, store::get_last_tick_at) {
            Ok(v) => {
                if v > 0.0 {
                    info!(
                        "[subconscious] resumed last_tick_at={v} from disk (situation report will only emit memory-tree rows newer than this)"
                    );
                }
                v
            }
            Err(e) => {
                warn!("[subconscious] last_tick_at load failed, falling back to 0.0: {e}");
                0.0
            }
        };

        Self {
            workspace_dir,
            interval_minutes: heartbeat.interval_minutes.max(5),
            context_budget_tokens: heartbeat.context_budget_tokens,
            enabled: heartbeat.enabled && heartbeat.inference_enabled,
            memory,
            state: Mutex::new(EngineState {
                last_tick_at,
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
            persist_last_tick_at(&self.workspace_dir, tick_at);
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

        // 3. Build situation report — memory-tree-derived sections (#623).
        let config_for_report = match crate::openhuman::config::Config::load_or_init().await {
            Ok(c) => c,
            Err(e) => {
                warn!("[subconscious] config load for situation report failed: {e}");
                // Without config we cannot read memory_tree tables — but we
                // can still build the env+tasks+reflections sections by
                // passing a default config. The signal sections will report
                // themselves as unavailable.
                crate::openhuman::config::Config::default()
            }
        };
        // Fetch last 8 reflections for anti-double-emit context.
        let recent_reflections = super::store::with_connection(&self.workspace_dir, |conn| {
            super::reflection_store::list_recent(conn, 8, None)
        })
        .unwrap_or_else(|e| {
            warn!("[subconscious] recent reflections load failed: {e}");
            Vec::new()
        });
        let report = build_situation_report(
            &config_for_report,
            &self.workspace_dir,
            last_tick_at,
            self.context_budget_tokens,
            &recent_reflections,
        )
        .await;

        // 4. Load identity context
        let identity = prompt::load_identity_context(&self.workspace_dir);

        // Release lock during LLM calls
        drop(state);

        // 5. Evaluate tasks + emit reflections via cloud chat (#623).
        let (evaluations, reflection_drafts) = self
            .evaluate_tasks_and_reflections(&due_tasks, &report, &identity)
            .await;

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

        // 6a. Persist reflections + post Notify ones (#623). Skipped on
        //     evaluation failure since the LLM didn't produce useful
        //     output anyway. We do NOT advance `last_tick_at` on
        //     failure, so the next tick sees the same window.
        if !evaluation_failed && !reflection_drafts.is_empty() {
            // Reuse the same `config_for_report` we built for the situation
            // report — the source-chunk resolver reads the same memory-tree
            // tables, so a single load is enough.
            persist_and_surface_reflections(
                &self.workspace_dir,
                &config_for_report,
                reflection_drafts,
                tick_at,
            )
            .await;
        }

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
            persist_last_tick_at(&self.workspace_dir, tick_at);
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
        let config_for_report = match crate::openhuman::config::Config::load_or_init().await {
            Ok(c) => c,
            Err(e) => {
                warn!("[subconscious] approve_escalation: config load failed: {e}");
                crate::openhuman::config::Config::default()
            }
        };
        let recent_reflections = super::store::with_connection(&self.workspace_dir, |conn| {
            super::reflection_store::list_recent(conn, 8, None)
        })
        .unwrap_or_default();
        let report = build_situation_report(
            &config_for_report,
            &self.workspace_dir,
            0.0, // fresh report for execution
            self.context_budget_tokens,
            &recent_reflections,
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

    /// Run the per-tick LLM call. Routes to cloud `summarization-v1` via
    /// the memory_tree chat provider (#623). On failure returns
    /// `(empty_evaluations, empty_drafts)` so `last_tick_at` is NOT
    /// advanced — the next tick re-fetches from the same point.
    async fn evaluate_tasks_and_reflections(
        &self,
        tasks: &[SubconsciousTask],
        report: &str,
        identity: &str,
    ) -> (Vec<TaskEvaluation>, Vec<ReflectionDraft>) {
        let prompt_text = prompt::build_evaluation_prompt(tasks, report, identity);

        let config = match crate::openhuman::config::Config::load_or_init().await {
            Ok(c) => c,
            Err(e) => {
                warn!("[subconscious] config load failed: {e}");
                return (
                    tasks
                        .iter()
                        .map(|t| TaskEvaluation {
                            task_id: t.id.clone(),
                            decision: TickDecision::Noop,
                            reason: format!("Evaluation failed: config load: {e}"),
                        })
                        .collect(),
                    vec![],
                );
            }
        };

        // Build the cloud chat provider. The subconscious tick uses
        // `ChatConsumer::Summarise` because the per-tick payload is
        // closer in shape to a structured-summary call than a per-chunk
        // entity extraction. No local fallback (per #623): if cloud is
        // unreachable, return empty results so the tick is treated as a
        // skip rather than a malformed advance.
        let provider: Arc<dyn ChatProvider> =
            match build_chat_provider(&config, ChatConsumer::Summarise) {
                Ok(p) => p,
                Err(e) => {
                    warn!("[subconscious] cloud chat provider init failed: {e}");
                    return (
                        tasks
                            .iter()
                            .map(|t| TaskEvaluation {
                                task_id: t.id.clone(),
                                decision: TickDecision::Noop,
                                reason: format!("Evaluation failed: provider init: {e}"),
                            })
                            .collect(),
                        vec![],
                    );
                }
            };

        let chat_prompt = ChatPrompt {
            system: prompt_text,
            user: "Evaluate the due tasks and surface reflections. Reply with JSON only."
                .to_string(),
            temperature: 0.0,
            kind: "subconscious_tick",
        };

        debug!(
            "[subconscious] cloud chat call provider={} tasks={}",
            provider.name(),
            tasks.len()
        );
        match provider.chat_for_json(&chat_prompt).await {
            Ok(raw) => {
                let (evals, drafts) = parse_response(&raw, tasks);
                debug!(
                    "[subconscious] cloud chat parsed evals={} drafts={}",
                    evals.len(),
                    drafts.len()
                );
                (evals, drafts)
            }
            Err(e) => {
                warn!("[subconscious] cloud chat failed (no local fallback): {e}");
                (
                    tasks
                        .iter()
                        .map(|t| TaskEvaluation {
                            task_id: t.id.clone(),
                            decision: TickDecision::Noop,
                            reason: format!("Evaluation failed: cloud chat: {e}"),
                        })
                        .collect(),
                    vec![],
                )
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

/// Parse the per-tick LLM response into evaluations + reflection drafts.
///
/// Best-effort: if the JSON has only `evaluations`, `reflections` is
/// empty; if it's a bare evaluations array, `reflections` is empty. If
/// nothing parses, all tasks default to Noop (with a parse-failure
/// reason) and `reflections` is empty.
fn parse_response(
    text: &str,
    tasks: &[SubconsciousTask],
) -> (Vec<TaskEvaluation>, Vec<ReflectionDraft>) {
    let json_text = extract_json(text);

    // 1. Full envelope (preferred).
    if let Ok(response) = serde_json::from_str::<EvaluationResponse>(json_text) {
        let evals = if response.evaluations.is_empty() {
            // The LLM returned only reflections — fall through to the
            // default-noop branch for tasks but keep reflections.
            tasks
                .iter()
                .map(|t| TaskEvaluation {
                    task_id: t.id.clone(),
                    decision: TickDecision::Noop,
                    reason: "No evaluation returned by model".to_string(),
                })
                .collect()
        } else {
            response.evaluations
        };
        return (evals, response.reflections);
    }

    // 2. Bare evaluations array (legacy shape pre-#623).
    if let Ok(evals) = serde_json::from_str::<Vec<TaskEvaluation>>(json_text) {
        if !evals.is_empty() {
            return (evals, vec![]);
        }
    }

    warn!("[subconscious] could not parse LLM response, defaulting all tasks to noop");
    let evals = tasks
        .iter()
        .map(|t| TaskEvaluation {
            task_id: t.id.clone(),
            decision: TickDecision::Noop,
            reason: "Unparseable evaluation response".to_string(),
        })
        .collect();
    (evals, vec![])
}

/// Persist a batch of LLM-emitted reflection drafts.
///
/// Caps to `MAX_REFLECTIONS_PER_TICK`. Failures on individual writes
/// are logged but do not abort the rest — the tick must finish even if
/// one row trips an I/O error.
///
/// Note: prior versions of this function also auto-posted `Notify`-
/// disposition reflections into a `system:subconscious` conversation
/// thread. That auto-post path is removed — reflections live exclusively
/// on the Intelligence tab. The user can spawn a fresh conversation from
/// any reflection via the `reflections_act` RPC (drives the action button).
async fn persist_and_surface_reflections(
    workspace_dir: &std::path::Path,
    config: &crate::openhuman::config::Config,
    drafts: Vec<ReflectionDraft>,
    now: f64,
) -> Vec<Reflection> {
    let (drafts, dropped) = apply_cap(drafts);
    if dropped > 0 {
        debug!(
            "[subconscious] reflections cap dropped {} excess (kept {})",
            dropped,
            drafts.len()
        );
    }
    if drafts.is_empty() {
        return vec![];
    }

    // Hydrate drafts into full reflections with fresh ids. For each draft,
    // resolve its `source_refs` against the live memory-tree data NOW so
    // the snapshot freezes the LLM's actual context. The chunks ride
    // alongside the reflection row and feed both the Intelligence-tab
    // "Sources" disclosure and the orchestrator's system-prompt memory-
    // context injection for any chat turn in a thread spawned from this
    // reflection. Resolver failures degrade per-chunk to empty content
    // (see `source_chunk::resolve_chunks`).
    let reflections: Vec<Reflection> = drafts
        .into_iter()
        .map(|d| {
            let chunks = resolve_chunks(config, &d.source_refs);
            hydrate_draft(d, uuid::Uuid::new_v4().to_string(), now, chunks)
        })
        .collect();

    // Persist all reflections in one connection. Idempotent inserts —
    // duplicate ids cannot occur here because we just generated them,
    // but the IGNORE clause makes a future retry safe.
    if let Err(e) = store::with_connection(workspace_dir, |conn| {
        for r in &reflections {
            if let Err(e) = reflection_store::add_reflection(conn, r) {
                warn!("[subconscious] reflection persist failed id={}: {e}", r.id);
            }
        }
        Ok(())
    }) {
        warn!("[subconscious] reflection batch persist failed: {e}");
    }

    reflections
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

/// Best-effort durability for the in-memory `last_tick_at` advance.
/// SQLite write failures are downgraded to a warning — the in-memory
/// value still advances and the current process keeps deduping
/// correctly. The next restart would just cold-start as before, which
/// is the pre-fix behaviour.
fn persist_last_tick_at(workspace_dir: &std::path::Path, tick_at: f64) {
    if let Err(e) =
        store::with_connection(workspace_dir, |conn| store::set_last_tick_at(conn, tick_at))
    {
        warn!("[subconscious] failed to persist last_tick_at={tick_at}: {e}");
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
