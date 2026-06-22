//! Subconscious engine — periodic agent loop that maintains a scratchpad.
//!
//! On each tick: load scratchpad → retrieve memory context (in code) →
//! build situation report → run subconscious agent (with tool access +
//! timeout) → agent maintains scratchpad via tools → log the run.
//!
//! ## Concurrency & timeouts
//!
//! A per-engine `tick_lock` prevents overlapping ticks. Each tick has
//! a hard wall-clock timeout (`TICK_TIMEOUT`) so a stuck LLM call
//! cannot block the loop forever. Individual tool calls within the
//! agent turn are bounded by the agent harness's own iteration cap.

use super::scratchpad;
use super::situation_report::build_situation_report;
use super::store;
use super::types::{SubconsciousStatus, TickResult};
use crate::openhuman::config::schema::SubconsciousMode;
use crate::openhuman::config::Config;
use crate::openhuman::credentials::{AuthService, APP_SESSION_PROVIDER};
use crate::openhuman::memory_store::MemoryClientRef;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// Max chunks to retrieve from memory before the LLM call.
const MEMORY_RETRIEVAL_MAX_CHUNKS: u32 = 30;

/// Hard timeout for a single subconscious tick (agent run).
const TICK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Per-tool-call timeout injected into the agent config.
const TOOL_CALL_TIMEOUT_SECS: u64 = 5 * 60;

/// Pick the `TrustedAutomationSource` variant for a subconscious tick.
///
/// Extracted from the engine's `run_agent` body so the
/// origin-escalation contract can be unit-tested without spinning up
/// a real `Agent` + provider.
///
/// Contract: any tick whose situation report contained third-party
/// sync content (Gmail / Slack / Notion / sealed source summaries)
/// must run with `SubconsciousTainted` so the approval gate refuses
/// external_effect tools. Untainted ticks keep the legacy
/// `Subconscious` origin.
pub(crate) fn tick_origin_source(
    has_external_content: bool,
) -> crate::openhuman::agent::turn_origin::TrustedAutomationSource {
    if has_external_content {
        crate::openhuman::agent::turn_origin::TrustedAutomationSource::SubconsciousTainted
    } else {
        crate::openhuman::agent::turn_origin::TrustedAutomationSource::Subconscious
    }
}

pub struct SubconsciousEngine {
    workspace_dir: PathBuf,
    mode: SubconsciousMode,
    interval_minutes: u32,
    context_budget_tokens: u32,
    enabled: bool,
    memory: Option<MemoryClientRef>,
    state: Mutex<EngineState>,
    tick_generation: AtomicU64,
    tick_lock: Mutex<()>,
}

struct EngineState {
    last_tick_at: f64,
    total_ticks: u64,
    consecutive_failures: u64,
    provider_unavailable_reason: Option<String>,
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
        let last_tick_at = match store::with_connection(&workspace_dir, store::get_last_tick_at) {
            Ok(v) => {
                if v > 0.0 {
                    info!("[subconscious] resumed last_tick_at={v} from disk");
                }
                v
            }
            Err(e) => {
                warn!("[subconscious] last_tick_at load failed, falling back to 0.0: {e}");
                0.0
            }
        };

        let mode = heartbeat.effective_subconscious_mode();

        Self {
            workspace_dir,
            mode,
            interval_minutes: mode.default_interval_minutes().max(5),
            context_budget_tokens: heartbeat.context_budget_tokens,
            enabled: mode.is_enabled(),
            memory,
            state: Mutex::new(EngineState {
                last_tick_at,
                total_ticks: 0,
                consecutive_failures: 0,
                provider_unavailable_reason: None,
            }),
            tick_generation: AtomicU64::new(0),
            tick_lock: Mutex::new(()),
        }
    }

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
                        "[subconscious] tick: duration={}ms response_chars={}",
                        result.duration_ms, result.response_chars
                    );
                }
                Err(e) => {
                    warn!("[subconscious] tick error: {e}");
                }
            }
        }
    }

    pub async fn tick(&self) -> Result<TickResult> {
        let _tick_guard =
            match tokio::time::timeout(std::time::Duration::from_secs(5), self.tick_lock.lock())
                .await
            {
                Ok(guard) => guard,
                Err(_) => {
                    warn!("[subconscious] tick skipped — another tick is still running");
                    return Ok(TickResult {
                        tick_at: now_secs(),
                        duration_ms: 0,
                        response_chars: 0,
                    });
                }
            };

        match tokio::time::timeout(TICK_TIMEOUT, self.tick_inner()).await {
            Ok(result) => result,
            Err(_) => {
                warn!(
                    "[subconscious] tick timed out after {}s",
                    TICK_TIMEOUT.as_secs()
                );
                let mut state = self.state.lock().await;
                state.consecutive_failures += 1;
                state.total_ticks += 1;
                Ok(TickResult {
                    tick_at: now_secs(),
                    duration_ms: TICK_TIMEOUT.as_millis() as u64,
                    response_chars: 0,
                })
            }
        }
    }

    async fn tick_inner(&self) -> Result<TickResult> {
        let started = std::time::Instant::now();
        let tick_at = now_secs();

        let my_generation = self.tick_generation.fetch_add(1, Ordering::SeqCst) + 1;

        let config = match Config::load_or_init().await {
            Ok(c) => c,
            Err(e) => {
                warn!("[subconscious] config load failed: {e}");
                let mut state = self.state.lock().await;
                state.provider_unavailable_reason = Some(format!("Config unavailable: {e}"));
                state.consecutive_failures += 1;
                state.total_ticks += 1;
                return Ok(TickResult {
                    tick_at,
                    duration_ms: started.elapsed().as_millis() as u64,
                    response_chars: 0,
                });
            }
        };

        if let Some(reason) = subconscious_provider_unavailable_reason(&config) {
            info!("[subconscious] provider unavailable, skipping tick: {reason}");
            let mut state = self.state.lock().await;
            state.provider_unavailable_reason = Some(reason);
            state.consecutive_failures += 1;
            state.total_ticks += 1;
            return Ok(TickResult {
                tick_at,
                duration_ms: started.elapsed().as_millis() as u64,
                response_chars: 0,
            });
        }

        let mut state = self.state.lock().await;
        state.provider_unavailable_reason = None;
        let last_tick_at = state.last_tick_at;
        drop(state);

        // 1. Build situation report
        let report = build_situation_report(
            &config,
            &self.workspace_dir,
            last_tick_at,
            self.context_budget_tokens,
        )
        .await;
        let has_external_content = report.has_external_content;

        // 2. Load scratchpad (persistent working memory)
        let scratchpad_entries = scratchpad::load(&self.workspace_dir).unwrap_or_else(|e| {
            warn!("[subconscious] scratchpad load failed: {e}");
            Vec::new()
        });
        let scratchpad_section = scratchpad::render_for_prompt(&scratchpad_entries);

        // 3. Pre-LLM memory retrieval — query the memory tree using
        //    scratchpad entries as context so the recall is focused on
        //    what the subconscious is currently tracking.
        let memory_section = retrieve_memory_context(&self.memory, &scratchpad_entries).await;

        // 4. Load identity context
        let identity = load_identity_context(&self.workspace_dir);

        // 5. Build user message with dynamic context (system prompt comes from agent definition)
        let agent_prompt = format!(
            "{identity}\n\n\
             ## Situation Report (pre-loaded context)\n\n\
             {situation}\n\n\
             {memory}\n\n\
             {scratchpad}",
            situation = report.prompt_text,
            memory = memory_section,
            scratchpad = scratchpad_section,
        );
        let agent_result = self
            .run_agent(&config, &agent_prompt, has_external_content)
            .await;
        let agent_failed = agent_result.is_err();
        let response_chars = match &agent_result {
            Ok(chars) => *chars,
            Err(_) => 0,
        };

        // 6. Check if superseded
        if self.tick_generation.load(Ordering::SeqCst) != my_generation {
            info!("[subconscious] tick superseded by newer tick, discarding");
            let mut state = self.state.lock().await;
            state.total_ticks += 1;
            return Ok(TickResult {
                tick_at,
                duration_ms: started.elapsed().as_millis() as u64,
                response_chars: 0,
            });
        }

        // 7. Update state — only advance last_tick_at and reset failures
        //    when the agent actually ran. Errors keep consecutive_failures
        //    incrementing and leave last_tick_at unchanged so the next tick
        //    re-fetches the same window.
        let mut state = self.state.lock().await;
        state.total_ticks += 1;
        if agent_failed {
            state.consecutive_failures += 1;
        } else {
            state.consecutive_failures = 0;
            state.last_tick_at = tick_at;
            persist_last_tick_at(&self.workspace_dir, tick_at);
        }

        Ok(TickResult {
            tick_at,
            duration_ms: started.elapsed().as_millis() as u64,
            response_chars,
        })
    }

    pub async fn status(&self) -> SubconsciousStatus {
        let state = self.state.lock().await;

        SubconsciousStatus {
            enabled: self.enabled,
            mode: self.mode.as_str().to_string(),
            provider_available: state.provider_unavailable_reason.is_none(),
            provider_unavailable_reason: state.provider_unavailable_reason.clone(),
            interval_minutes: self.interval_minutes,
            last_tick_at: if state.last_tick_at > 0.0 {
                Some(state.last_tick_at)
            } else {
                None
            },
            total_ticks: state.total_ticks,
            consecutive_failures: state.consecutive_failures,
        }
    }

    /// Run the subconscious agent with mode-appropriate tool access.
    /// The agent maintains the scratchpad via tools during its turn.
    /// Returns `response_chars` on success, or `Err` on agent init/run failure.
    async fn run_agent(
        &self,
        config: &Config,
        prompt_text: &str,
        has_external_content: bool,
    ) -> Result<usize, String> {
        use crate::openhuman::agent::Agent;

        let mut effective = config.clone();
        effective.agent.agent_timeout_secs = TOOL_CALL_TIMEOUT_SECS;
        match self.mode {
            SubconsciousMode::Simple => {
                effective.autonomy.level = crate::openhuman::security::AutonomyLevel::ReadOnly;
                effective.agent.max_tool_iterations = 15;
            }
            SubconsciousMode::Aggressive | SubconsciousMode::EventDriven => {
                effective.autonomy.level = crate::openhuman::security::AutonomyLevel::Full;
                effective.agent.max_tool_iterations = 30;
            }
            SubconsciousMode::Off => return Ok(0),
        }

        let mut agent = Agent::from_config(&effective).map_err(|e| {
            warn!("[subconscious] agent init failed: {e}");
            format!("agent init: {e}")
        })?;

        agent.set_event_context(
            format!("subconscious:tick:{}", now_secs() as u64),
            "subconscious",
        );

        let mode_guidance = match self.mode {
            SubconsciousMode::Aggressive => {
                "\n\n\
                You are in AGGRESSIVE mode. You may use `spawn_subagent` to delegate \
                complex tasks:\n\
                - `agent_id: \"orchestrator\"` with `model: \"reasoning-v1\"` for deep \
                  reasoning and multi-step execution\n\
                - `agent_id: \"researcher\"` for web research and external data\n\n\
                Use this power when you identify actionable opportunities, approaching \
                deadlines, or patterns that warrant proactive help."
            }
            _ => "",
        };

        let user_message = format!(
            "{prompt_text}\n\n\
             ## Instructions\n\n\
             Based on the situation report and your existing scratchpad, maintain your \
             scratchpad — add new observations, edit stale entries, remove resolved items.\n\n\
             Your scratchpad IS your continuity mechanism across ticks. Keep it focused \
             and actionable.\
             {mode_guidance}",
        );

        debug!("[subconscious] spawning agent with tool access");
        let source = tick_origin_source(has_external_content);
        debug!(
            "[subconscious] tick origin source={:?} has_external_content={has_external_content}",
            source
        );
        let origin = crate::openhuman::agent::turn_origin::AgentTurnOrigin::TrustedAutomation {
            job_id: format!("subconscious:tick:{}", now_secs() as u64),
            source,
        };
        let response = crate::openhuman::agent::turn_origin::with_origin(
            origin,
            agent.run_single(&user_message),
        )
        .await
        .map_err(|e| {
            warn!("[subconscious] agent run failed: {e}");
            format!("agent run: {e}")
        })?;

        let response_chars = response.chars().count();
        info!(
            "[subconscious] agent completed (response {} chars)",
            response_chars
        );
        Ok(response_chars)
    }
}

// ── Provider routing ────────────────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq)]
enum SubconsciousProviderRoute {
    LocalOllama { model: String },
    OpenHumanCloud,
    Other(String),
}

pub(crate) fn subconscious_provider_unavailable_reason(config: &Config) -> Option<String> {
    match resolve_subconscious_route(config) {
        SubconsciousProviderRoute::LocalOllama { .. } => None,
        SubconsciousProviderRoute::OpenHumanCloud => {
            if crate::openhuman::scheduler_gate::is_signed_out() {
                return Some(
                    "Sign in to use the OpenHuman cloud Subconscious provider.".to_string(),
                );
            }

            let state_dir = config
                .config_path
                .parent()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| config.workspace_dir.clone());
            let auth = AuthService::new(&state_dir, config.secrets.encrypt);
            match auth.get_provider_bearer_token(APP_SESSION_PROVIDER, None) {
                Ok(Some(token)) if !token.trim().is_empty() => None,
                Ok(_) => Some(
                    "Sign in or configure a local Subconscious provider in Settings > AI."
                        .to_string(),
                ),
                Err(e) => Some(format!("Unable to read the OpenHuman session: {e}")),
            }
        }
        SubconsciousProviderRoute::Other(_) => None,
    }
}

fn resolve_subconscious_route(config: &Config) -> SubconsciousProviderRoute {
    if let Some(model) = config.workload_local_model("subconscious") {
        return SubconsciousProviderRoute::LocalOllama { model };
    }

    let raw = config
        .subconscious_provider
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("cloud");
    let is_openhuman_cloud = raw.eq_ignore_ascii_case("cloud")
        || raw.eq_ignore_ascii_case("openhuman")
        || raw.to_ascii_lowercase().starts_with("openhuman:");
    if is_openhuman_cloud {
        SubconsciousProviderRoute::OpenHumanCloud
    } else {
        SubconsciousProviderRoute::Other(raw.to_string())
    }
}

// ── Pre-LLM memory retrieval ────────────────────────────────────────────────

/// Query the memory tree using scratchpad entries as context, returning
/// a rendered markdown section to inject into the user message. This
/// replaces the old `call_memory_agent` tool call — the retrieval now
/// happens in code before the LLM runs, saving a full agent turn.
async fn retrieve_memory_context(
    memory: &Option<MemoryClientRef>,
    scratchpad_entries: &[scratchpad::ScratchpadEntry],
) -> String {
    let client = match memory {
        Some(c) => c,
        None => {
            debug!("[subconscious] no memory client — skipping pre-LLM retrieval");
            return String::new();
        }
    };

    // Build a query from high-priority scratchpad items (p5+) or fall back
    // to a generic recent-activity query.
    let query = build_memory_query(scratchpad_entries);
    debug!(
        "[subconscious] pre-LLM memory retrieval query_len={}",
        query.len()
    );

    let started = std::time::Instant::now();

    // Query conversation_memory namespace for relevant context
    let conversation_ctx = client
        .query_namespace("conversation_memory", &query, MEMORY_RETRIEVAL_MAX_CHUNKS)
        .await
        .unwrap_or_else(|e| {
            warn!("[subconscious] conversation_memory query failed: {e}");
            String::new()
        });

    // Also recall recent learning reflections (user patterns, preferences)
    let reflections_ctx = client
        .recall_namespace("learning_reflections", 10)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();

    let elapsed = started.elapsed();
    info!(
        "[subconscious] pre-LLM memory retrieval done in {:.1}s conv_chars={} refl_chars={}",
        elapsed.as_secs_f64(),
        conversation_ctx.len(),
        reflections_ctx.len()
    );

    if conversation_ctx.is_empty() && reflections_ctx.is_empty() {
        return String::new();
    }

    let mut section = String::from("## Memory Context (pre-loaded)\n\n");
    if !conversation_ctx.is_empty() {
        section.push_str("### Recent Conversations & Activity\n\n");
        section.push_str(&conversation_ctx);
        section.push_str("\n\n");
    }
    if !reflections_ctx.is_empty() {
        section.push_str("### Learned User Patterns\n\n");
        section.push_str(&reflections_ctx);
        section.push_str("\n\n");
    }
    section
}

/// Build a memory query from scratchpad entries. High-priority items
/// (p5+) get included verbatim; lower-priority items contribute keywords.
/// Falls back to a generic query when the scratchpad is empty.
fn build_memory_query(entries: &[scratchpad::ScratchpadEntry]) -> String {
    if entries.is_empty() {
        return "What has the user been working on recently? Any upcoming deadlines, \
                unresolved threads, or notable activity across all sources?"
            .to_string();
    }

    let high_priority: Vec<&scratchpad::ScratchpadEntry> =
        entries.iter().filter(|e| e.priority >= 5).collect();

    if high_priority.is_empty() {
        // Use all entries as a broad query
        let bodies: Vec<&str> = entries.iter().map(|e| e.body.as_str()).collect();
        return format!(
            "Recent activity and updates related to: {}",
            bodies.join("; ")
        );
    }

    let bodies: Vec<&str> = high_priority.iter().map(|e| e.body.as_str()).collect();
    format!(
        "Updates and context for these tracked items: {}",
        bodies.join("; ")
    )
}

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

// ── Identity loading ───────────────────────────────────────────────────────

const IDENTITY_EXCERPT_CHARS: usize = 2000;

fn load_identity_context(workspace_dir: &std::path::Path) -> String {
    let prompts_dir = resolve_prompts_dir(workspace_dir);
    let mut ctx = String::new();

    if let Some(ref dir) = prompts_dir {
        if let Some(soul) = load_file_excerpt(dir, "SOUL.md") {
            ctx.push_str(&soul);
            ctx.push_str("\n\n");
        }
    }

    if let Some(profile) = load_file_excerpt(workspace_dir, "PROFILE.md") {
        ctx.push_str("## User Profile\n\n");
        ctx.push_str(&profile);
        ctx.push_str("\n\n");
    }

    if ctx.is_empty() {
        "You are OpenHuman, an AI assistant for productivity and collaboration.".to_string()
    } else {
        ctx
    }
}

fn resolve_prompts_dir(workspace_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let workspace_ai = workspace_dir.join("ai");
    if workspace_ai.is_dir() {
        return Some(workspace_ai);
    }

    if let Some(dir) = option_env!("CARGO_MANIFEST_DIR").map(std::path::PathBuf::from) {
        let candidate = dir
            .join("src")
            .join("openhuman")
            .join("agent")
            .join("prompts");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        return crate::openhuman::dev_paths::repo_ai_prompts_dir(&cwd);
    }

    None
}

fn load_file_excerpt(dir: &std::path::Path, filename: &str) -> Option<String> {
    let content = std::fs::read_to_string(dir.join(filename)).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() > IDENTITY_EXCERPT_CHARS {
        let truncated: String = trimmed.chars().take(IDENTITY_EXCERPT_CHARS).collect();
        Some(format!("{truncated}\n[... truncated]"))
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
