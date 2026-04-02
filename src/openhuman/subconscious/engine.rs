//! Subconscious loop engine — periodic background awareness.
//!
//! Replaces the old heartbeat engine with context-aware reasoning:
//! assembles a delta-based situation report, evaluates with the local
//! model, and decides whether to act, escalate, or do nothing.

use super::decision_log::DecisionLog;
use super::prompt::build_subconscious_prompt;
use super::situation_report::build_situation_report;
use super::types::{Decision, SubconsciousStatus, TickOutput, TickResult};
use crate::openhuman::config::Config;
use crate::openhuman::memory::{MemoryClient, MemoryClientRef};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{self, Duration};
use tracing::{debug, info, warn};

/// Memory namespace for storing subconscious state (decision log, etc.).
const SUBCONSCIOUS_NAMESPACE: &str = "subconscious";
/// Memory key for the persisted decision log.
const DECISION_LOG_KEY: &str = "__decision_log";

pub struct SubconsciousEngine {
    workspace_dir: PathBuf,
    interval_minutes: u32,
    context_budget_tokens: u32,
    enabled: bool,
    memory: Option<MemoryClientRef>,
    state: Arc<Mutex<EngineState>>,
}

struct EngineState {
    last_tick_at: f64,
    decision_log: DecisionLog,
    total_ticks: u64,
    total_escalations: u64,
}

impl SubconsciousEngine {
    /// Create from the top-level Config (reads config.heartbeat).
    pub fn new(config: &Config, memory: Option<MemoryClientRef>) -> Self {
        Self::from_heartbeat_config(&config.heartbeat, config.workspace_dir.clone(), memory)
    }

    /// Create directly from HeartbeatConfig (used by HeartbeatEngine).
    pub fn from_heartbeat_config(
        heartbeat: &crate::openhuman::config::HeartbeatConfig,
        workspace_dir: std::path::PathBuf,
        memory: Option<MemoryClientRef>,
    ) -> Self {
        Self {
            workspace_dir,
            interval_minutes: heartbeat.interval_minutes.max(5),
            context_budget_tokens: heartbeat.context_budget_tokens,
            enabled: heartbeat.enabled && heartbeat.inference_enabled,
            memory,
            state: Arc::new(Mutex::new(EngineState {
                last_tick_at: 0.0,
                decision_log: DecisionLog::new(),
                total_ticks: 0,
                total_escalations: 0,
            })),
        }
    }

    /// Start the subconscious loop (runs until cancelled).
    pub async fn run(&self) -> Result<()> {
        if !self.enabled {
            info!("[subconscious] disabled, exiting");
            return Ok(());
        }

        info!(
            "[subconscious] started: every {} minutes, budget {} tokens",
            self.interval_minutes, self.context_budget_tokens
        );

        // Load persisted decision log from memory
        self.load_decision_log().await;

        let mut interval =
            time::interval(Duration::from_secs(u64::from(self.interval_minutes) * 60));

        loop {
            interval.tick().await;

            match self.tick().await {
                Ok(result) => {
                    info!(
                        "[subconscious] tick complete: decision={:?} reason=\"{}\" duration={}ms",
                        result.output.decision, result.output.reason, result.duration_ms
                    );
                }
                Err(e) => {
                    warn!("[subconscious] tick error: {e}");
                }
            }
        }
    }

    /// Execute a single subconscious tick. Public for manual triggering via RPC.
    ///
    /// The entire tick holds the state lock to prevent concurrent ticks
    /// from duplicating work (fixes CodeRabbit #1: serialize executions).
    pub async fn tick(&self) -> Result<TickResult> {
        let started = std::time::Instant::now();
        let tick_at = now_secs();

        // Hold the lock for the entire tick to prevent concurrent execution
        let mut state = self.state.lock().await;

        // Load persisted decision log if this is the first tick (fixes #5)
        if state.total_ticks == 0 {
            if let Some(ref memory) = self.memory {
                if let Ok(Some(value)) = memory
                    .kv_get(Some(SUBCONSCIOUS_NAMESPACE), DECISION_LOG_KEY)
                    .await
                {
                    if let Some(json) = value.as_str() {
                        if let Ok(log) = DecisionLog::from_json(json) {
                            state.decision_log = log;
                            debug!("[subconscious] loaded persisted decision log");
                        }
                    }
                }
            }
        }

        state.decision_log.prune_expired();
        let last_tick_at = state.last_tick_at;

        // 1. Read HEARTBEAT.md tasks
        let tasks = read_heartbeat_tasks(&self.workspace_dir).await;
        if tasks.is_empty() {
            debug!("[subconscious] HEARTBEAT.md empty or missing, skipping tick");
            state.last_tick_at = tick_at;
            state.total_ticks += 1;
            return Ok(TickResult {
                tick_at,
                output: TickOutput {
                    decision: Decision::Noop,
                    reason: "No tasks in HEARTBEAT.md".to_string(),
                    actions: vec![],
                },
                source_doc_ids: vec![],
                duration_ms: started.elapsed().as_millis() as u64,
                tokens_used: 0,
            });
        }

        debug!(
            "[subconscious] {} heartbeat tasks, assembling state (last_tick={:.0})",
            tasks.len(),
            last_tick_at
        );

        // 2. Assemble current state (delta since last tick)
        let memory_ref = self.memory.as_ref().map(|m| m.as_ref());
        let (report, new_doc_ids) = build_situation_report_with_doc_ids(
            memory_ref,
            &self.workspace_dir,
            last_tick_at,
            self.context_budget_tokens,
        )
        .await;

        // 3. Filter out already-surfaced doc IDs (fixes #3: dedup)
        let unsurfaced_doc_ids = state.decision_log.filter_unsurfaced(&new_doc_ids);
        let has_new_data = !unsurfaced_doc_ids.is_empty();

        // 4. Check if there's actually new state to evaluate
        let has_memory_changes = report.contains("new/updated");
        let has_changes = has_new_data || has_memory_changes;

        let output = if !has_changes {
            debug!("[subconscious] no actionable changes, skipping inference");
            TickOutput {
                decision: Decision::Noop,
                reason: "No new state changes since last tick.".to_string(),
                actions: vec![],
            }
        } else {
            // 5. Build task-driven prompt and call local model
            let prompt = build_subconscious_prompt(&tasks, &report);
            debug!(
                "[subconscious] calling local model ({} tasks, {} new docs)",
                tasks.len(),
                unsurfaced_doc_ids.len()
            );
            // Release lock during LLM call (it's slow)
            drop(state);
            let result = self.evaluate_with_local_model(&prompt).await?;
            state = self.state.lock().await;
            result
        };

        // 6. Update state
        state.last_tick_at = tick_at;
        state.total_ticks += 1;

        // 7. Record decision with actual doc IDs (fixes #3: dedup)
        if output.decision != Decision::Noop {
            state
                .decision_log
                .record(tick_at, &output, unsurfaced_doc_ids.clone());

            if output.decision == Decision::Escalate {
                state.total_escalations += 1;
            }
        }

        let duration_ms = started.elapsed().as_millis() as u64;
        drop(state);

        // 8. Persist decision log
        self.save_decision_log().await;

        // 9. Handle actions — always store as RecommendedAction JSON (fixes #4)
        if !output.actions.is_empty() {
            if let Ok(json) = serde_json::to_string(&output.actions) {
                self.store_actions(&json).await;
            }
        }
        if output.decision == Decision::Escalate {
            self.handle_escalation(&output, &report).await;
        }

        Ok(TickResult {
            tick_at,
            output,
            source_doc_ids: unsurfaced_doc_ids,
            duration_ms,
            tokens_used: 0,
        })
    }

    /// Get current status.
    pub async fn status(&self) -> SubconsciousStatus {
        let state = self.state.lock().await;
        SubconsciousStatus {
            enabled: self.enabled,
            interval_minutes: self.interval_minutes,
            last_tick_at: if state.last_tick_at > 0.0 {
                Some(state.last_tick_at)
            } else {
                None
            },
            last_decision: state
                .decision_log
                .records()
                .last()
                .map(|r| r.decision.clone()),
            total_ticks: state.total_ticks,
            total_escalations: state.total_escalations,
        }
    }

    /// Evaluate the situation report using the local AI model (Ollama).
    async fn evaluate_with_local_model(&self, prompt: &str) -> Result<TickOutput> {
        let config = crate::openhuman::config::Config::load_or_init()
            .await
            .map_err(|e| anyhow::anyhow!("load config: {e}"))?;

        let messages = vec![
            crate::openhuman::local_ai::ops::LocalAiChatMessage {
                role: "system".to_string(),
                content: prompt.to_string(),
            },
            crate::openhuman::local_ai::ops::LocalAiChatMessage {
                role: "user".to_string(),
                content:
                    "Evaluate the situation report and respond with ONLY the JSON decision object."
                        .to_string(),
            },
        ];

        match crate::openhuman::local_ai::ops::local_ai_chat(&config, messages, None).await {
            Ok(outcome) => {
                let text = outcome.value;
                debug!("[subconscious] local model response: {text}");
                parse_tick_output(&text)
            }
            Err(e) => {
                warn!("[subconscious] local model inference failed: {e}, falling back to noop");
                Ok(TickOutput {
                    decision: Decision::Noop,
                    reason: format!("Local model inference failed: {e}"),
                    actions: vec![],
                })
            }
        }
    }

    /// Handle escalation — call the stronger model to resolve into concrete actions.
    async fn handle_escalation(&self, output: &TickOutput, situation_report: &str) {
        info!(
            "[subconscious] ESCALATION: {} — calling agent for resolution",
            output.reason
        );

        let escalation_prompt = format!(
            "The subconscious background loop detected something important:\n\n\
             Reason: {}\n\n\
             Situation report:\n{}\n\n\
             Based on this, what concrete actions should be taken? \
             Respond with a JSON object:\n\
             {{\"actions\": [{{\"type\": \"notify|store_memory|run_tool\", \"description\": \"what to do\", \"priority\": \"low|medium|high\"}}]}}",
            output.reason, situation_report
        );

        let config = match crate::openhuman::config::Config::load_or_init().await {
            Ok(c) => c,
            Err(e) => {
                warn!("[subconscious] escalation failed — could not load config: {e}");
                return;
            }
        };

        match crate::openhuman::local_ai::ops::agent_chat_simple(
            &config,
            &escalation_prompt,
            config.heartbeat.escalation_model.clone(),
            Some(0.3),
        )
        .await
        {
            Ok(outcome) => {
                info!("[subconscious] escalation resolved");
                // Normalize agent response to RecommendedAction format (fixes #4)
                let actions = normalize_escalation_response(&outcome.value, &output.reason);
                if let Ok(json) = serde_json::to_string(&actions) {
                    self.store_actions(&json).await;
                }
            }
            Err(e) => {
                warn!("[subconscious] escalation agent call failed: {e}");
                // Fall back: store the original actions from local model
                if let Ok(json) = serde_json::to_string(&output.actions) {
                    self.store_actions(&json).await;
                }
            }
        }
    }

    /// Store action results in the subconscious memory namespace for the UI to consume.
    async fn store_actions(&self, content: &str) {
        if let Some(ref memory) = self.memory {
            // Use millisecond precision + random suffix to avoid key collisions (fixes #7)
            let timestamp_ms = (now_secs() * 1000.0) as u64;
            let suffix = rand_suffix();
            let key = format!("actions:{timestamp_ms}:{suffix}");
            let value = serde_json::Value::String(content.to_string());
            if let Err(e) = memory
                .kv_set(Some(SUBCONSCIOUS_NAMESPACE), &key, &value)
                .await
            {
                warn!("[subconscious] failed to store actions: {e}");
            } else {
                debug!("[subconscious] actions stored as {key}");
            }
        }
    }

    /// Load decision log from memory.
    async fn load_decision_log(&self) {
        if let Some(ref memory) = self.memory {
            match memory
                .kv_get(Some(SUBCONSCIOUS_NAMESPACE), DECISION_LOG_KEY)
                .await
            {
                Ok(Some(value)) => {
                    if let Some(json) = value.as_str() {
                        match DecisionLog::from_json(json) {
                            Ok(log) => {
                                let mut state = self.state.lock().await;
                                state.decision_log = log;
                                debug!("[subconscious] loaded decision log from memory");
                            }
                            Err(e) => {
                                warn!("[subconscious] failed to parse decision log: {e}");
                            }
                        }
                    }
                }
                Ok(None) => {
                    debug!("[subconscious] no persisted decision log found");
                }
                Err(e) => {
                    warn!("[subconscious] failed to load decision log: {e}");
                }
            }
        }
    }

    /// Save decision log to memory.
    async fn save_decision_log(&self) {
        if let Some(ref memory) = self.memory {
            let state = self.state.lock().await;
            match state.decision_log.to_json() {
                Ok(json) => {
                    let value = serde_json::Value::String(json);
                    if let Err(e) = memory
                        .kv_set(Some(SUBCONSCIOUS_NAMESPACE), DECISION_LOG_KEY, &value)
                        .await
                    {
                        warn!("[subconscious] failed to save decision log: {e}");
                    }
                }
                Err(e) => {
                    warn!("[subconscious] failed to serialize decision log: {e}");
                }
            }
        }
    }
}

/// Parse the local model's JSON response into a TickOutput.
fn parse_tick_output(text: &str) -> Result<TickOutput> {
    // Try direct JSON parse first
    if let Ok(output) = serde_json::from_str::<TickOutput>(text) {
        return Ok(output);
    }

    // Try extracting JSON from markdown code blocks
    let trimmed = text.trim();
    if let Some(json_start) = trimmed.find('{') {
        if let Some(json_end) = trimmed.rfind('}') {
            let json_slice = &trimmed[json_start..=json_end];
            if let Ok(output) = serde_json::from_str::<TickOutput>(json_slice) {
                return Ok(output);
            }
        }
    }

    warn!("[subconscious] could not parse model output as JSON, defaulting to noop");
    Ok(TickOutput {
        decision: Decision::Noop,
        reason: format!("Unparseable model output: {}", &text[..text.len().min(100)]),
        actions: vec![],
    })
}

/// Normalize the agent's escalation response into RecommendedAction format.
/// Ensures consistent schema regardless of what the agent returns.
fn normalize_escalation_response(
    agent_response: &str,
    reason: &str,
) -> Vec<super::types::RecommendedAction> {
    // Try parsing as JSON with an actions array
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(agent_response) {
        if let Some(actions) = value.get("actions").and_then(|a| a.as_array()) {
            let mut result = Vec::new();
            for action in actions {
                if let Ok(ra) =
                    serde_json::from_value::<super::types::RecommendedAction>(action.clone())
                {
                    result.push(ra);
                }
            }
            if !result.is_empty() {
                return result;
            }
        }
    }

    // Fallback: wrap the raw response as a single notify action
    vec![super::types::RecommendedAction {
        action_type: super::types::ActionType::EscalateToAgent,
        description: format!(
            "Escalation resolved: {}",
            &agent_response[..agent_response.len().min(300)]
        ),
        priority: super::types::Priority::High,
        task: Some(reason.to_string()),
    }]
}

/// Generate a short random suffix for KV key uniqueness.
fn rand_suffix() -> u32 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut hasher);
    std::thread::current().id().hash(&mut hasher);
    (hasher.finish() % 10000) as u32
}

/// Wrapper around build_situation_report that also returns new doc IDs.
async fn build_situation_report_with_doc_ids(
    memory: Option<&super::super::memory::MemoryClient>,
    workspace_dir: &std::path::Path,
    last_tick_at: f64,
    token_budget: u32,
) -> (String, Vec<String>) {
    let report = build_situation_report(memory, workspace_dir, last_tick_at, token_budget).await;

    // Extract doc IDs from memory if available
    let mut doc_ids = Vec::new();
    if let Some(client) = memory {
        if let Ok(docs) = client.list_documents(None).await {
            let is_cold_start = last_tick_at <= 0.0;
            if let Some(arr) = docs
                .as_array()
                .or_else(|| docs.get("documents").and_then(|v| v.as_array()))
            {
                for doc in arr {
                    let updated_at = doc
                        .get("updated_at")
                        .or_else(|| doc.get("updatedAt"))
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);
                    if is_cold_start || updated_at > last_tick_at {
                        if let Some(id) = doc
                            .get("document_id")
                            .or_else(|| doc.get("documentId"))
                            .and_then(|v| v.as_str())
                        {
                            doc_ids.push(id.to_string());
                        }
                    }
                }
            }
        }
    }

    (report, doc_ids)
}

/// Read tasks from HEARTBEAT.md in the workspace.
async fn read_heartbeat_tasks(workspace_dir: &std::path::Path) -> Vec<String> {
    let path = workspace_dir.join("HEARTBEAT.md");
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- ").map(ToString::to_string))
        .filter(|s| !s.is_empty())
        .collect()
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_noop() {
        let output = parse_tick_output(
            r#"{"decision": "noop", "reason": "Nothing changed.", "actions": []}"#,
        )
        .unwrap();
        assert_eq!(output.decision, Decision::Noop);
    }

    #[test]
    fn parse_valid_escalate() {
        let output = parse_tick_output(
            r#"{"decision": "escalate", "reason": "Deadline moved to tomorrow", "actions": [{"type": "escalate_to_agent", "description": "Notify about deadline change", "priority": "high"}]}"#,
        )
        .unwrap();
        assert_eq!(output.decision, Decision::Escalate);
        assert_eq!(output.actions.len(), 1);
    }

    #[test]
    fn parse_json_in_markdown_block() {
        let output = parse_tick_output(
            "```json\n{\"decision\": \"act\", \"reason\": \"Store to memory\", \"actions\": []}\n```",
        )
        .unwrap();
        assert_eq!(output.decision, Decision::Act);
    }

    #[test]
    fn parse_garbage_falls_back_to_noop() {
        let output = parse_tick_output("This is not JSON at all").unwrap();
        assert_eq!(output.decision, Decision::Noop);
        assert!(output.reason.contains("Unparseable"));
    }
}
