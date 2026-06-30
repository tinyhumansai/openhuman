//! Live harness audit for reusable async sub-agent delegation.
//!
//! This binary intentionally uses the user's real OpenHuman config and live
//! provider/backend credentials. It records only sanitized progress metadata:
//! tool names, task/session ids, statuses, character counts, and elapsed times.
//! It does not print prompts, tool arguments, assistant replies, transcripts,
//! credentials, or integration payloads.
//!
//! Typical usage:
//!
//! ```sh
//! scripts/debug/harness-subagent-audit.sh --turns 2
//! ```

use std::collections::BTreeSet;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::Parser;
use openhuman_core::openhuman::agent::harness::run_queue::QueueMode;
use openhuman_core::openhuman::agent::progress::AgentProgress;
use openhuman_core::openhuman::agent::Agent;
use openhuman_core::openhuman::agent_orchestration::{
    running_subagents,
    subagent_sessions::{DurableSubagentSession, DurableSubagentStatus, SubagentSessionStore},
};
use openhuman_core::openhuman::config::Config;
use serde::Serialize;
use tokio::sync::mpsc;

#[derive(Parser, Debug)]
#[command(name = "harness-subagent-audit")]
struct Args {
    /// Sub-agent archetype to request from the orchestrator.
    #[arg(long, default_value = "researcher")]
    agent_id: String,

    /// Stable reusable task key. Defaults to audit-subagent-<unix-seconds>.
    #[arg(long)]
    task_key: Option<String>,

    /// Number of parent turns to run. Use 2 to audit same-key reuse.
    #[arg(long, default_value_t = 2)]
    turns: usize,

    /// Seconds to wait for the durable session to appear or settle.
    #[arg(long, default_value_t = 45)]
    wait_secs: u64,

    /// Require the final durable session status to leave running.
    #[arg(long)]
    require_completion: bool,

    /// Override the first parent prompt. The audit task key is not appended.
    #[arg(long)]
    prompt: Option<String>,

    /// Override the second parent prompt. Only used when --turns is at least 2.
    #[arg(long)]
    follow_up: Option<String>,

    /// Print sanitized JSON summary in addition to the human summary.
    #[arg(long)]
    json: bool,

    /// After the first async sub-agent spawn, steer the running child through its run queue.
    #[arg(long)]
    steer_mid_run: bool,

    /// Delay after SubagentSpawned before attempting the steer.
    #[arg(long, default_value_t = 250)]
    steer_delay_ms: u64,

    /// Seconds to retry resolving/registering the running child before steering fails.
    #[arg(long, default_value_t = 10)]
    steer_wait_secs: u64,

    /// Override the steering message. The message itself is never printed.
    #[arg(long)]
    steer_message: Option<String>,
}

#[derive(Debug, Default, Serialize)]
struct ProgressStats {
    parent_tool_started: Vec<ParentToolStarted>,
    parent_tool_completed: Vec<ParentToolCompleted>,
    subagent_spawned: Vec<SubagentSpawnedEvent>,
    subagent_completed: Vec<SubagentCompletedEvent>,
    subagent_failed: Vec<SubagentFailedEvent>,
    subagent_tool_started: Vec<SubagentToolEvent>,
    subagent_tool_completed: Vec<SubagentToolCompletedEvent>,
    steer_attempts: Vec<SteerAttemptEvent>,
    turn_completed: usize,
}

#[derive(Debug, Serialize)]
struct ParentToolStarted {
    turn: usize,
    call_id: String,
    tool_name: String,
    iteration: u32,
    argument_keys: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ParentToolCompleted {
    turn: usize,
    call_id: String,
    tool_name: String,
    success: bool,
    output_chars: usize,
    elapsed_ms: u64,
    iteration: u32,
}

#[derive(Debug, Clone, Serialize)]
struct SubagentSpawnedEvent {
    turn: usize,
    agent_id: String,
    task_id: String,
    mode: String,
    dedicated_thread: bool,
    prompt_chars: usize,
    worker_thread_id: Option<String>,
    display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct SteerAttemptEvent {
    turn: usize,
    agent_id: String,
    task_id: String,
    subagent_session_id: Option<String>,
    delivered: bool,
    error: Option<String>,
    attempts: usize,
    elapsed_ms: u128,
    message_chars: usize,
}

#[derive(Clone)]
struct SteerAuditConfig {
    store: SubagentSessionStore,
    task_key: String,
    message: String,
    delay: Duration,
    wait_for: Duration,
    fired: Arc<AtomicBool>,
}

#[derive(Debug, Serialize)]
struct SubagentCompletedEvent {
    turn: usize,
    agent_id: String,
    task_id: String,
    elapsed_ms: u64,
    iterations: u32,
    output_chars: usize,
}

#[derive(Debug, Serialize)]
struct SubagentFailedEvent {
    turn: usize,
    agent_id: String,
    task_id: String,
    error_chars: usize,
}

#[derive(Debug, Serialize)]
struct SubagentToolEvent {
    turn: usize,
    agent_id: String,
    task_id: String,
    call_id: String,
    tool_name: String,
    iteration: u32,
}

#[derive(Debug, Serialize)]
struct SubagentToolCompletedEvent {
    turn: usize,
    agent_id: String,
    task_id: String,
    call_id: String,
    tool_name: String,
    success: bool,
    output_chars: usize,
    elapsed_ms: u64,
    iteration: u32,
}

#[derive(Debug, Clone, Serialize)]
struct SessionSummary {
    subagent_session_id: String,
    parent_session: String,
    parent_thread_id: Option<String>,
    worker_thread_id: Option<String>,
    agent_id: String,
    display_name: Option<String>,
    task_key: String,
    current_task_id: Option<String>,
    status: DurableSubagentStatus,
    reusable: bool,
    created_at: String,
    updated_at: String,
    last_used_at: String,
}

#[derive(Debug, Serialize)]
struct AuditSummary {
    task_key: String,
    agent_id: String,
    turns_requested: usize,
    assistant_reply_chars: Vec<usize>,
    progress: ProgressStats,
    sessions: Vec<SessionSummary>,
    checks: Vec<AuditCheck>,
    passed: bool,
}

#[derive(Debug, Serialize)]
struct AuditCheck {
    name: &'static str,
    passed: bool,
    detail: String,
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("[harness_subagent_audit] ERROR: {err:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();
    let task_key = args
        .task_key
        .clone()
        .unwrap_or_else(|| format!("audit-subagent-{}", unix_seconds()));

    eprintln!("[harness_subagent_audit] loading live OpenHuman config");
    let config = Config::load_or_init()
        .await
        .context("loading user config (Config::load_or_init)")?;
    let store = SubagentSessionStore::new(config.workspace_dir.clone());
    eprintln!(
        "[harness_subagent_audit] workspace_dir={} session_store={}",
        config.workspace_dir.display(),
        store.path().display()
    );
    eprintln!(
        "[harness_subagent_audit] default_model={:?} dispatcher={:?}",
        config.default_model, config.agent.tool_dispatcher
    );

    let before_sessions = load_matching_sessions(&store, &task_key)
        .context("loading existing matching durable subagent sessions")?;
    if !before_sessions.is_empty() {
        eprintln!(
            "[harness_subagent_audit] found {} pre-existing session(s) for task_key={}; reuse checks may include prior state",
            before_sessions.len(),
            task_key
        );
    }

    let mut agent = Agent::from_config(&config).context("Agent::from_config failed")?;
    eprintln!("[harness_subagent_audit] fetching connected integrations");
    agent.fetch_connected_integrations().await;
    let refreshed = agent.refresh_delegation_tools();
    eprintln!(
        "[harness_subagent_audit] connected_integrations={} delegation_tools_refreshed={} visible_tools={} model={}",
        agent.connected_integrations().len(),
        refreshed,
        agent.tools().len(),
        agent.model_name()
    );

    let stats = Arc::new(Mutex::new(ProgressStats::default()));
    let current_turn = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = mpsc::channel(512);
    agent.set_on_progress(Some(tx));
    let steer_config = args.steer_mid_run.then(|| SteerAuditConfig {
        store: store.clone(),
        task_key: task_key.clone(),
        message: args
            .steer_message
            .clone()
            .unwrap_or_else(|| default_steer_message(&task_key)),
        delay: Duration::from_millis(args.steer_delay_ms),
        wait_for: Duration::from_secs(args.steer_wait_secs),
        fired: Arc::new(AtomicBool::new(false)),
    });
    let progress_task = tokio::spawn(drain_progress(
        rx,
        stats.clone(),
        current_turn.clone(),
        steer_config,
    ));

    let turns = args.turns.clamp(1, 2);
    let mut assistant_reply_chars = Vec::new();
    for turn in 1..=turns {
        current_turn.store(turn, Ordering::SeqCst);
        let prompt = if turn == 1 {
            args.prompt
                .clone()
                .unwrap_or_else(|| first_turn_prompt(&args.agent_id, &task_key))
        } else {
            args.follow_up
                .clone()
                .unwrap_or_else(|| second_turn_prompt(&args.agent_id, &task_key))
        };
        eprintln!(
            "[harness_subagent_audit] >>> parent_turn={} prompt_chars={} task_key={}",
            turn,
            prompt.chars().count(),
            task_key
        );
        let started = std::time::Instant::now();
        let reply = agent
            .run_single(&prompt)
            .await
            .with_context(|| format!("agent.run_single failed on turn {turn}"))?;
        eprintln!(
            "[harness_subagent_audit] <<< parent_turn={} elapsed_ms={} assistant_reply_chars={}",
            turn,
            started.elapsed().as_millis(),
            reply.chars().count()
        );
        assistant_reply_chars.push(reply.chars().count());
    }

    let sessions = poll_matching_sessions(
        &store,
        &task_key,
        Duration::from_secs(args.wait_secs),
        args.require_completion,
    )
    .await
    .context("polling durable subagent sessions")?;

    agent.set_on_progress(None);
    drop(agent);
    let _ = tokio::time::timeout(Duration::from_secs(2), progress_task).await;

    let progress = take_stats(stats);
    let checks = evaluate_checks(
        &args.agent_id,
        turns,
        args.require_completion,
        &progress,
        &sessions,
    );
    let passed = checks.iter().all(|check| check.passed);
    let summary = AuditSummary {
        task_key,
        agent_id: args.agent_id,
        turns_requested: turns,
        assistant_reply_chars,
        progress,
        sessions,
        checks,
        passed,
    };

    print_human_summary(&summary);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    }

    if !summary.passed {
        std::process::exit(1);
    }
    Ok(())
}

async fn drain_progress(
    mut rx: mpsc::Receiver<AgentProgress>,
    stats: Arc<Mutex<ProgressStats>>,
    current_turn: Arc<AtomicUsize>,
    steer_config: Option<SteerAuditConfig>,
) {
    while let Some(event) = rx.recv().await {
        let turn = current_turn.load(Ordering::SeqCst);
        match event {
            AgentProgress::ToolCallStarted {
                call_id,
                tool_name,
                arguments,
                iteration,
                ..
            } => {
                let argument_keys = argument_keys(&arguments);
                eprintln!(
                    "[harness_subagent_audit] progress turn={} parent_tool_started tool={} call_id={} iteration={} argument_keys={:?}",
                    turn, tool_name, call_id, iteration, argument_keys
                );
                stats
                    .lock()
                    .expect("progress stats mutex poisoned")
                    .parent_tool_started
                    .push(ParentToolStarted {
                        turn,
                        call_id,
                        tool_name,
                        iteration,
                        argument_keys,
                    });
            }
            AgentProgress::ToolCallCompleted {
                call_id,
                tool_name,
                success,
                output_chars,
                elapsed_ms,
                iteration,
            } => {
                eprintln!(
                    "[harness_subagent_audit] progress turn={} parent_tool_completed tool={} call_id={} success={} output_chars={} elapsed_ms={} iteration={}",
                    turn, tool_name, call_id, success, output_chars, elapsed_ms, iteration
                );
                stats
                    .lock()
                    .expect("progress stats mutex poisoned")
                    .parent_tool_completed
                    .push(ParentToolCompleted {
                        turn,
                        call_id,
                        tool_name,
                        success,
                        output_chars,
                        elapsed_ms,
                        iteration,
                    });
            }
            AgentProgress::SubagentSpawned {
                agent_id,
                task_id,
                mode,
                dedicated_thread,
                prompt_chars,
                worker_thread_id,
                display_name,
            } => {
                eprintln!(
                    "[harness_subagent_audit] progress turn={} subagent_spawned agent_id={} task_id={} mode={} dedicated_thread={} prompt_chars={} worker_thread_id={}",
                    turn,
                    agent_id,
                    task_id,
                    mode,
                    dedicated_thread,
                    prompt_chars,
                    worker_thread_id.as_deref().unwrap_or("none")
                );
                let spawned = SubagentSpawnedEvent {
                    turn,
                    agent_id,
                    task_id,
                    mode,
                    dedicated_thread,
                    prompt_chars,
                    worker_thread_id,
                    display_name,
                };
                stats
                    .lock()
                    .expect("progress stats mutex poisoned")
                    .subagent_spawned
                    .push(spawned.clone());
                if let Some(config) = steer_config.as_ref() {
                    if config
                        .fired
                        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
                    {
                        let config = config.clone();
                        let stats = stats.clone();
                        tokio::spawn(async move {
                            let attempt = steer_after_spawn(config, spawned).await;
                            eprintln!(
                                "[harness_subagent_audit] steer_attempt turn={} task_id={} delivered={} attempts={} elapsed_ms={} message_chars={} error={}",
                                attempt.turn,
                                attempt.task_id,
                                attempt.delivered,
                                attempt.attempts,
                                attempt.elapsed_ms,
                                attempt.message_chars,
                                attempt.error.as_deref().unwrap_or("none")
                            );
                            stats
                                .lock()
                                .expect("progress stats mutex poisoned")
                                .steer_attempts
                                .push(attempt);
                        });
                    }
                }
            }
            AgentProgress::SubagentCompleted {
                agent_id,
                task_id,
                elapsed_ms,
                iterations,
                output_chars,
                ..
            } => {
                eprintln!(
                    "[harness_subagent_audit] progress turn={} subagent_completed agent_id={} task_id={} elapsed_ms={} iterations={} output_chars={}",
                    turn, agent_id, task_id, elapsed_ms, iterations, output_chars
                );
                stats
                    .lock()
                    .expect("progress stats mutex poisoned")
                    .subagent_completed
                    .push(SubagentCompletedEvent {
                        turn,
                        agent_id,
                        task_id,
                        elapsed_ms,
                        iterations,
                        output_chars,
                    });
            }
            AgentProgress::SubagentFailed {
                agent_id,
                task_id,
                error,
            } => {
                eprintln!(
                    "[harness_subagent_audit] progress turn={} subagent_failed agent_id={} task_id={} error_chars={}",
                    turn,
                    agent_id,
                    task_id,
                    error.chars().count()
                );
                stats
                    .lock()
                    .expect("progress stats mutex poisoned")
                    .subagent_failed
                    .push(SubagentFailedEvent {
                        turn,
                        agent_id,
                        task_id,
                        error_chars: error.chars().count(),
                    });
            }
            AgentProgress::SubagentToolCallStarted {
                agent_id,
                task_id,
                call_id,
                tool_name,
                arguments: _,
                iteration,
                ..
            } => {
                eprintln!(
                    "[harness_subagent_audit] progress turn={} subagent_tool_started agent_id={} task_id={} tool={} call_id={} iteration={}",
                    turn, agent_id, task_id, tool_name, call_id, iteration
                );
                stats
                    .lock()
                    .expect("progress stats mutex poisoned")
                    .subagent_tool_started
                    .push(SubagentToolEvent {
                        turn,
                        agent_id,
                        task_id,
                        call_id,
                        tool_name,
                        iteration,
                    });
            }
            AgentProgress::SubagentToolCallCompleted {
                agent_id,
                task_id,
                call_id,
                tool_name,
                success,
                output_chars,
                output: _,
                elapsed_ms,
                iteration,
            } => {
                eprintln!(
                    "[harness_subagent_audit] progress turn={} subagent_tool_completed agent_id={} task_id={} tool={} call_id={} success={} output_chars={} elapsed_ms={} iteration={}",
                    turn, agent_id, task_id, tool_name, call_id, success, output_chars, elapsed_ms, iteration
                );
                stats
                    .lock()
                    .expect("progress stats mutex poisoned")
                    .subagent_tool_completed
                    .push(SubagentToolCompletedEvent {
                        turn,
                        agent_id,
                        task_id,
                        call_id,
                        tool_name,
                        success,
                        output_chars,
                        elapsed_ms,
                        iteration,
                    });
            }
            AgentProgress::TurnCompleted { .. } => {
                stats
                    .lock()
                    .expect("progress stats mutex poisoned")
                    .turn_completed += 1;
            }
            AgentProgress::SubagentAwaitingUser {
                agent_id,
                task_id,
                question,
                worker_thread_id,
            } => {
                eprintln!(
                    "[harness_subagent_audit] progress turn={} subagent_awaiting_user agent_id={} task_id={} question_chars={} worker_thread_id={}",
                    turn,
                    agent_id,
                    task_id,
                    question.chars().count(),
                    worker_thread_id.as_deref().unwrap_or("none")
                );
            }
            AgentProgress::IterationStarted { .. }
            | AgentProgress::SubagentIterationStarted { .. }
            | AgentProgress::TextDelta { .. }
            | AgentProgress::ThinkingDelta { .. }
            | AgentProgress::SubagentTextDelta { .. }
            | AgentProgress::SubagentThinkingDelta { .. }
            | AgentProgress::ToolCallArgsDelta { .. }
            | AgentProgress::TaskBoardUpdated { .. }
            | AgentProgress::TurnCostUpdated { .. }
            | AgentProgress::TurnStarted => {}
        }
    }
}

async fn steer_after_spawn(
    config: SteerAuditConfig,
    spawned: SubagentSpawnedEvent,
) -> SteerAttemptEvent {
    tokio::time::sleep(config.delay).await;
    let started = std::time::Instant::now();
    let mut attempts = 0;
    loop {
        attempts += 1;
        let attempt_error;
        match find_session_for_task(&config.store, &config.task_key, &spawned.task_id) {
            Ok(Some(session)) => {
                match running_subagents::steer(
                    &spawned.task_id,
                    &session.parent_session,
                    config.message.clone(),
                    QueueMode::Steer,
                )
                .await
                {
                    Ok(()) => {
                        return SteerAttemptEvent {
                            turn: spawned.turn,
                            agent_id: spawned.agent_id,
                            task_id: spawned.task_id,
                            subagent_session_id: Some(session.subagent_session_id),
                            delivered: true,
                            error: None,
                            attempts,
                            elapsed_ms: started.elapsed().as_millis(),
                            message_chars: config.message.chars().count(),
                        };
                    }
                    Err(err) => {
                        attempt_error = Some(format!("{err:?}"));
                        if !matches!(err, running_subagents::SteerError::Unknown) {
                            return failed_steer_attempt(
                                spawned,
                                attempt_error,
                                attempts,
                                started.elapsed().as_millis(),
                                config.message.chars().count(),
                            );
                        }
                    }
                }
            }
            Ok(None) => {
                attempt_error = Some("durable session not found yet".to_string());
            }
            Err(err) => {
                attempt_error = Some(err.to_string());
            }
        }

        if started.elapsed() >= config.wait_for {
            return failed_steer_attempt(
                spawned,
                attempt_error,
                attempts,
                started.elapsed().as_millis(),
                config.message.chars().count(),
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn failed_steer_attempt(
    spawned: SubagentSpawnedEvent,
    error: Option<String>,
    attempts: usize,
    elapsed_ms: u128,
    message_chars: usize,
) -> SteerAttemptEvent {
    SteerAttemptEvent {
        turn: spawned.turn,
        agent_id: spawned.agent_id,
        task_id: spawned.task_id,
        subagent_session_id: None,
        delivered: false,
        error,
        attempts,
        elapsed_ms,
        message_chars,
    }
}

fn find_session_for_task(
    store: &SubagentSessionStore,
    task_key: &str,
    task_id: &str,
) -> Result<Option<SessionSummary>> {
    Ok(store
        .load()
        .map_err(anyhow::Error::msg)?
        .into_iter()
        .filter(|session| session.task_key == task_key)
        .find(|session| session.current_task_id.as_deref() == Some(task_id))
        .map(SessionSummary::from))
}

fn argument_keys(value: &serde_json::Value) -> Vec<String> {
    value
        .as_object()
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default()
}

fn first_turn_prompt(agent_id: &str, task_key: &str) -> String {
    format!(
        "Harness audit run. Call spawn_subagent exactly once with agent_id `{agent_id}`, \
         task_key `{task_key}`, blocking false, and fresh false. The delegated prompt should ask \
         the sub-agent to return a concise confirmation for audit marker `{task_key}` without \
         asking for clarification. After the tool returns, answer with a brief note that the \
         async reusable worker was started. Do not call wait_subagent in this turn."
    )
}

fn second_turn_prompt(agent_id: &str, task_key: &str) -> String {
    format!(
        "Harness audit follow-up. Continue the same reusable sub-agent by calling spawn_subagent \
         exactly once with agent_id `{agent_id}`, the same task_key `{task_key}`, blocking false, \
         and fresh false. The delegated prompt should add one short follow-up instruction for \
         audit marker `{task_key}`. After the tool returns, answer briefly. Do not call \
         wait_subagent in this turn."
    )
}

fn default_steer_message(task_key: &str) -> String {
    format!(
        "Mid-run steering audit for marker `{task_key}`: acknowledge that this instruction arrived through the async steering queue, then keep the final answer concise."
    )
}

async fn poll_matching_sessions(
    store: &SubagentSessionStore,
    task_key: &str,
    wait_for: Duration,
    require_completion: bool,
) -> Result<Vec<SessionSummary>> {
    let started = std::time::Instant::now();
    loop {
        let sessions = load_matching_sessions(store, task_key)?;
        let settled = sessions.iter().any(|session| {
            !require_completion || !matches!(session.status, DurableSubagentStatus::Running)
        });
        if !sessions.is_empty() && settled {
            return Ok(sessions);
        }
        if started.elapsed() >= wait_for {
            return Ok(sessions);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn load_matching_sessions(
    store: &SubagentSessionStore,
    task_key: &str,
) -> Result<Vec<SessionSummary>> {
    let sessions = store.load().map_err(anyhow::Error::msg).with_context(|| {
        format!(
            "loading durable subagent store at {}",
            store.path().display()
        )
    })?;
    Ok(sessions
        .into_iter()
        .filter(|session| session.task_key == task_key)
        .map(SessionSummary::from)
        .collect())
}

impl From<DurableSubagentSession> for SessionSummary {
    fn from(session: DurableSubagentSession) -> Self {
        Self {
            subagent_session_id: session.subagent_session_id,
            parent_session: session.parent_session,
            parent_thread_id: session.parent_thread_id,
            worker_thread_id: session.worker_thread_id,
            agent_id: session.agent_id,
            display_name: session.display_name,
            task_key: session.task_key,
            current_task_id: session.current_task_id,
            status: session.status,
            reusable: session.reusable,
            created_at: session.created_at,
            updated_at: session.updated_at,
            last_used_at: session.last_used_at,
        }
    }
}

fn evaluate_checks(
    agent_id: &str,
    turns: usize,
    require_completion: bool,
    progress: &ProgressStats,
    sessions: &[SessionSummary],
) -> Vec<AuditCheck> {
    let mut checks = Vec::new();
    let parent_spawn_calls = progress
        .parent_tool_started
        .iter()
        .filter(|event| is_spawn_tool(&event.tool_name))
        .count();
    checks.push(AuditCheck {
        name: "parent_called_spawn_tool",
        passed: parent_spawn_calls >= turns,
        detail: format!(
            "observed {parent_spawn_calls} spawn_subagent/spawn_async_subagent start event(s)"
        ),
    });

    if !progress.steer_attempts.is_empty() {
        let delivered = progress
            .steer_attempts
            .iter()
            .filter(|attempt| attempt.delivered)
            .count();
        checks.push(AuditCheck {
            name: "mid_run_steer_delivered",
            passed: delivered > 0,
            detail: format!(
                "observed {delivered} delivered steer attempt(s) out of {}",
                progress.steer_attempts.len()
            ),
        });
    }

    let completed_spawn_calls = progress
        .parent_tool_completed
        .iter()
        .filter(|event| is_spawn_tool(&event.tool_name) && event.success)
        .count();
    checks.push(AuditCheck {
        name: "spawn_tool_completed_successfully",
        passed: completed_spawn_calls >= turns,
        detail: format!(
            "observed {completed_spawn_calls} successful spawn tool completion event(s)"
        ),
    });

    let spawned_events = progress
        .subagent_spawned
        .iter()
        .filter(|event| event.agent_id == agent_id && event.mode == "async")
        .count();
    checks.push(AuditCheck {
        name: "async_subagent_registered",
        passed: spawned_events > 0 || !sessions.is_empty(),
        detail: format!(
            "observed {spawned_events} async SubagentSpawned event(s), {} persisted matching session(s)",
            sessions.len()
        ),
    });

    checks.push(AuditCheck {
        name: "durable_session_persisted",
        passed: !sessions.is_empty(),
        detail: format!("persisted matching session count={}", sessions.len()),
    });

    let unique_session_ids: BTreeSet<_> = sessions
        .iter()
        .map(|session| session.subagent_session_id.as_str())
        .collect();
    checks.push(AuditCheck {
        name: "single_reusable_session_for_task_key",
        passed: turns < 2 || unique_session_ids.len() == 1,
        detail: format!(
            "unique matching subagent_session_id count={}",
            unique_session_ids.len()
        ),
    });

    let session_agent_ok = sessions
        .iter()
        .all(|session| session.agent_id == agent_id && session.reusable);
    checks.push(AuditCheck {
        name: "session_agent_and_reusable_flag_match",
        passed: !sessions.is_empty() && session_agent_ok,
        detail: format!(
            "all sessions match agent_id={agent_id} and reusable=true: {session_agent_ok}"
        ),
    });

    let running_count = sessions
        .iter()
        .filter(|session| matches!(session.status, DurableSubagentStatus::Running))
        .count();
    checks.push(AuditCheck {
        name: "completion_requirement",
        passed: !require_completion || running_count == 0,
        detail: if require_completion {
            format!("running matching sessions after wait={running_count}")
        } else {
            "not required".to_string()
        },
    });

    checks
}

fn is_spawn_tool(tool_name: &str) -> bool {
    tool_name == "spawn_subagent" || tool_name == "spawn_async_subagent"
}

fn take_stats(stats: Arc<Mutex<ProgressStats>>) -> ProgressStats {
    match Arc::try_unwrap(stats) {
        Ok(mutex) => mutex.into_inner().expect("progress stats mutex poisoned"),
        Err(stats) => std::mem::take(&mut *stats.lock().expect("progress stats mutex poisoned")),
    }
}

fn print_human_summary(summary: &AuditSummary) {
    println!("=== Harness Subagent Audit ===");
    println!("task_key: {}", summary.task_key);
    println!("agent_id: {}", summary.agent_id);
    println!("turns_requested: {}", summary.turns_requested);
    println!("assistant_reply_chars: {:?}", summary.assistant_reply_chars);
    println!(
        "progress: parent_spawn_started={} parent_spawn_completed={} subagent_spawned={} subagent_completed={} subagent_failed={} subagent_tool_started={} subagent_tool_completed={}",
        summary
            .progress
            .parent_tool_started
            .iter()
            .filter(|event| is_spawn_tool(&event.tool_name))
            .count(),
        summary
            .progress
            .parent_tool_completed
            .iter()
            .filter(|event| is_spawn_tool(&event.tool_name))
            .count(),
        summary.progress.subagent_spawned.len(),
        summary.progress.subagent_completed.len(),
        summary.progress.subagent_failed.len(),
        summary.progress.subagent_tool_started.len(),
        summary.progress.subagent_tool_completed.len()
    );
    if !summary.progress.steer_attempts.is_empty() {
        for attempt in &summary.progress.steer_attempts {
            println!(
                "steer: task_id={} delivered={} attempts={} elapsed_ms={} message_chars={} error={}",
                attempt.task_id,
                attempt.delivered,
                attempt.attempts,
                attempt.elapsed_ms,
                attempt.message_chars,
                attempt.error.as_deref().unwrap_or("none")
            );
        }
    }
    println!("sessions:");
    if summary.sessions.is_empty() {
        println!("  none");
    } else {
        for session in &summary.sessions {
            println!(
                "  subagent_session_id={} task_id={} status={:?} reusable={} worker_thread_id={} updated_at={}",
                session.subagent_session_id,
                session.current_task_id.as_deref().unwrap_or("none"),
                session.status,
                session.reusable,
                session.worker_thread_id.as_deref().unwrap_or("none"),
                session.updated_at
            );
        }
    }
    println!("checks:");
    for check in &summary.checks {
        println!(
            "  [{}] {} - {}",
            if check.passed { "pass" } else { "fail" },
            check.name,
            check.detail
        );
    }
    println!("verdict: {}", if summary.passed { "PASS" } else { "FAIL" });
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
