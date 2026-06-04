//! Subconscious engine — periodic agent loop that produces thoughts.
//!
//! On each tick: build situation report → run subconscious agent →
//! parse thoughts from output → create thread → store reflections.

use super::prompt;
use super::reflection::{apply_cap, hydrate_draft, Reflection, ReflectionDraft};
use super::reflection_store;
use super::situation_report::build_situation_report;
use super::source_chunk::resolve_chunks;
use super::store;
use super::types::{SubconsciousStatus, TickResult};
use crate::openhuman::config::schema::SubconsciousMode;
use crate::openhuman::config::Config;
use crate::openhuman::credentials::{AuthService, APP_SESSION_PROVIDER};
use crate::openhuman::memory_store::MemoryClientRef;
use anyhow::Result;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

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
                        "[subconscious] tick: thoughts={} thread={:?} duration={}ms",
                        result.thoughts_count, result.thread_id, result.duration_ms
                    );
                }
                Err(e) => {
                    warn!("[subconscious] tick error: {e}");
                }
            }
        }
    }

    pub async fn tick(&self) -> Result<TickResult> {
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
                    thoughts_count: 0,
                    thread_id: None,
                    duration_ms: started.elapsed().as_millis() as u64,
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
                thoughts_count: 0,
                thread_id: None,
                duration_ms: started.elapsed().as_millis() as u64,
            });
        }

        let mut state = self.state.lock().await;
        state.provider_unavailable_reason = None;
        let last_tick_at = state.last_tick_at;
        drop(state);

        // 1. Build situation report
        let recent_reflections = store::with_connection(&self.workspace_dir, |conn| {
            reflection_store::list_recent(conn, 8, None)
        })
        .unwrap_or_else(|e| {
            warn!("[subconscious] recent reflections load failed: {e}");
            Vec::new()
        });
        let report = build_situation_report(
            &config,
            &self.workspace_dir,
            last_tick_at,
            self.context_budget_tokens,
            &recent_reflections,
        )
        .await;
        let has_external_content = report.has_external_content;

        // 2. Load identity context
        let identity = prompt::load_identity_context(&self.workspace_dir);

        // 3. Run the subconscious agent
        let agent_prompt = prompt::build_agent_prompt(&report.prompt_text, &identity);
        let agent_result = self
            .run_agent(&config, &agent_prompt, has_external_content)
            .await;
        let agent_failed = agent_result.is_err();
        let drafts = agent_result.unwrap_or_default();

        // 4. Check if superseded
        if self.tick_generation.load(Ordering::SeqCst) != my_generation {
            info!("[subconscious] tick superseded by newer tick, discarding");
            let mut state = self.state.lock().await;
            state.total_ticks += 1;
            return Ok(TickResult {
                tick_at,
                thoughts_count: 0,
                thread_id: None,
                duration_ms: started.elapsed().as_millis() as u64,
            });
        }

        // 5. Create thread and persist reflections
        let thread_id = if !drafts.is_empty() {
            let tid = self.create_tick_thread(&config, tick_at, &agent_prompt, &drafts);
            Some(tid)
        } else {
            None
        };

        let reflections = persist_reflections(
            &self.workspace_dir,
            &config,
            drafts,
            tick_at,
            thread_id.as_deref(),
        )
        .await;

        let thoughts_count = reflections.len();

        // 6. Update state — only advance last_tick_at and reset failures
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
            thoughts_count,
            thread_id,
            duration_ms: started.elapsed().as_millis() as u64,
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

    /// Run the subconscious agent with mode-appropriate tool access and
    /// parse thoughts from its final response. Returns `Err` on agent
    /// init/run failure so the caller can track consecutive failures
    /// separately from an empty-but-successful tick.
    async fn run_agent(
        &self,
        config: &Config,
        prompt_text: &str,
        has_external_content: bool,
    ) -> Result<Vec<ReflectionDraft>, String> {
        use crate::openhuman::agent::Agent;

        let mut effective = config.clone();
        match self.mode {
            SubconsciousMode::Simple => {
                effective.autonomy.level = crate::openhuman::security::AutonomyLevel::ReadOnly;
                effective.agent.max_tool_iterations = 4;
            }
            SubconsciousMode::Aggressive => {
                effective.autonomy.level = crate::openhuman::security::AutonomyLevel::Full;
                effective.agent.max_tool_iterations = 8;
            }
            SubconsciousMode::Off => return Ok(vec![]),
        }

        let mut agent = Agent::from_config(&effective).map_err(|e| {
            warn!("[subconscious] agent init failed: {e}");
            format!("agent init: {e}")
        })?;

        agent.set_event_context(
            format!("subconscious:tick:{}", now_secs() as u64),
            "subconscious",
        );

        let user_message = format!(
            "{prompt_text}\n\n\
             Use your tools to look up any relevant memory, recent activity, or \
             context that would help you produce insightful observations. When \
             you're done researching, end your final message with a JSON block \
             of your thoughts:\n\n\
             ```json\n\
             {{\"thoughts\": [...]}}\n\
             ```"
        );

        debug!("[subconscious] spawning agent with tool access");
        // Subconscious ticks are trusted automation: the user enabled the
        // background loop knowing it should think about their state.
        // When the situation report carries content derived from third-
        // party sync sources (Gmail / Slack / Notion / sealed source
        // summaries), escalate the origin so the approval gate refuses
        // external_effect tools for the rest of the tick — a hostile
        // upstream message can otherwise nudge the LLM into a tool call
        // the user would never have authorised.
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

        let drafts = parse_thoughts(&response);
        info!(
            "[subconscious] agent produced {} thoughts (response {} chars)",
            drafts.len(),
            response.len()
        );
        Ok(drafts)
    }

    /// Create a conversation thread for this tick so the user can view the
    /// agent's reasoning by clicking on any thought.
    fn create_tick_thread(
        &self,
        config: &Config,
        tick_at: f64,
        _agent_prompt: &str,
        drafts: &[ReflectionDraft],
    ) -> String {
        let thread_id = uuid::Uuid::new_v4().to_string();
        let dt = chrono::DateTime::from_timestamp(tick_at as i64, 0)
            .unwrap_or_else(|| chrono::Utc::now());
        let thread_title = format!("Subconscious — {}", dt.format("%b %d, %H:%M"));
        let now_iso = chrono::Utc::now().to_rfc3339();

        if let Err(e) = crate::openhuman::memory_conversations::ensure_thread(
            config.workspace_dir.clone(),
            crate::openhuman::memory_conversations::CreateConversationThread {
                id: thread_id.clone(),
                title: thread_title,
                created_at: now_iso.clone(),
                parent_thread_id: None,
                labels: Some(vec!["subconscious".to_string()]),
                personality_id: None,
            },
        ) {
            warn!("[subconscious] failed to create tick thread: {e}");
            return thread_id;
        }

        // Seed thread with a summary of the thoughts as the assistant message
        let body = drafts
            .iter()
            .map(|d| {
                let action = d
                    .proposed_action
                    .as_deref()
                    .map(|a| format!("\n\n_Proposed action_: {a}"))
                    .unwrap_or_default();
                format!(
                    "**{}** — {}{}",
                    d.kind.as_str().replace('_', " "),
                    d.body,
                    action
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        let seed_message = crate::openhuman::memory_conversations::ConversationMessage {
            id: uuid::Uuid::new_v4().to_string(),
            content: body,
            message_type: "text".to_string(),
            extra_metadata: serde_json::json!({
                "origin": "subconscious_tick",
                "tick_at": tick_at,
                "thoughts_count": drafts.len(),
            }),
            sender: "assistant".to_string(),
            created_at: now_iso,
        };
        if let Err(e) = crate::openhuman::memory_conversations::append_message(
            config.workspace_dir.clone(),
            &thread_id,
            seed_message,
        ) {
            warn!("[subconscious] failed to seed tick thread: {e}");
        }

        thread_id
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

// ── Thought parsing ─────────────────────────────────────────────────────────

/// Response envelope for the agent's JSON output.
#[derive(Debug, Clone, serde::Deserialize)]
struct ThoughtsResponse {
    #[serde(default)]
    thoughts: Vec<ReflectionDraft>,
    // Backward compat: also accept "reflections" key
    #[serde(default)]
    reflections: Vec<ReflectionDraft>,
}

fn parse_thoughts(text: &str) -> Vec<ReflectionDraft> {
    let json_text = extract_json(text);

    // Try full envelope
    if let Ok(response) = serde_json::from_str::<ThoughtsResponse>(json_text) {
        let mut drafts = response.thoughts;
        if drafts.is_empty() {
            drafts = response.reflections;
        }
        if !drafts.is_empty() {
            return drafts;
        }
    }

    // Try bare array
    if let Ok(drafts) = serde_json::from_str::<Vec<ReflectionDraft>>(json_text) {
        if !drafts.is_empty() {
            return drafts;
        }
    }

    warn!("[subconscious] could not parse agent output for thoughts");
    vec![]
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

// ── Reflection persistence ──────────────────────────────────────────────────

async fn persist_reflections(
    workspace_dir: &std::path::Path,
    config: &Config,
    drafts: Vec<ReflectionDraft>,
    now: f64,
    thread_id: Option<&str>,
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

    let reflections: Vec<Reflection> = drafts
        .into_iter()
        .map(|d| {
            let chunks = resolve_chunks(config, &d.source_refs);
            hydrate_draft(
                d,
                uuid::Uuid::new_v4().to_string(),
                now,
                chunks,
                thread_id.map(String::from),
            )
        })
        .collect();

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
