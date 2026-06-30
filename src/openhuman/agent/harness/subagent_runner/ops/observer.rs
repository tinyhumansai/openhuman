//! Sub-agent [`TurnObserver`] implementation.
//!
//! Accumulates usage stats, persists per-iteration transcripts, and
//! mirrors assistant intents / tool results / final responses to the
//! spawn's worker thread (when one is attached).

use crate::openhuman::inference::provider::ChatMessage;
use crate::openhuman::memory_conversations::ConversationMessage;

use super::super::super::session::transcript;
use super::loop_::AggregatedUsage;

pub(super) struct SubagentObserver {
    pub(super) worker_thread_id: Option<String>,
    pub(super) workspace_dir: std::path::PathBuf,
    pub(super) transcript_stem: String,
    pub(super) agent_id: String,
    pub(super) task_id: String,
    pub(super) force_text_mode: bool,
    pub(super) usage: AggregatedUsage,
    /// Provenance + usage of the sub-agent's most recent provider call. Persisted
    /// onto the transcript's last assistant message (carries provider + model so
    /// per-thread usage reads can price the sub-agent at its actual model).
    pub(super) last_turn_usage: Option<transcript::TurnUsage>,
}

impl SubagentObserver {
    pub(super) fn append_worker_message(
        &self,
        content: String,
        sender: String,
        extra_metadata: serde_json::Value,
    ) {
        let Some(ref thread_id) = self.worker_thread_id else {
            return;
        };
        let message = ConversationMessage {
            id: format!("{}:{}", sender, uuid::Uuid::new_v4()),
            content,
            message_type: "text".to_string(),
            extra_metadata,
            sender,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Err(err) = crate::openhuman::memory_conversations::append_message(
            self.workspace_dir.clone(),
            thread_id,
            message,
        ) {
            tracing::debug!(
                agent_id = %self.agent_id,
                thread_id = %thread_id,
                error = %err,
                "[subagent_runner] failed to append message to worker thread"
            );
        }
    }

    pub(super) fn persist_transcript(&self, history: &[ChatMessage]) {
        let path = match transcript::resolve_keyed_transcript_path(
            &self.workspace_dir,
            &self.transcript_stem,
        ) {
            Ok(p) => p,
            Err(err) => {
                tracing::debug!(
                    agent_id = %self.agent_id,
                    error = %err,
                    "[subagent_runner] failed to resolve transcript path"
                );
                return;
            }
        };
        let now = chrono::Utc::now().to_rfc3339();
        let meta = transcript::TranscriptMeta {
            agent_name: self.agent_id.clone(),
            agent_id: Some(self.agent_id.clone()),
            agent_type: Some("subagent".to_string()),
            dispatcher: "native".into(),
            provider: self
                .last_turn_usage
                .as_ref()
                .map(|usage| usage.provider.clone()),
            model: self
                .last_turn_usage
                .as_ref()
                .map(|usage| usage.model.clone()),
            created: now.clone(),
            updated: now,
            turn_count: 1,
            input_tokens: self.usage.input_tokens,
            output_tokens: self.usage.output_tokens,
            cached_input_tokens: self.usage.cached_input_tokens,
            charged_amount_usd: self.usage.charged_amount_usd,
            thread_id: crate::openhuman::inference::provider::thread_context::current_thread_id(),
            task_id: Some(self.task_id.clone()),
        };
        if let Err(err) =
            transcript::write_transcript(&path, history, &meta, self.last_turn_usage.as_ref())
        {
            tracing::debug!(
                agent_id = %self.agent_id,
                error = %err,
                "[subagent_runner] failed to write transcript"
            );
        }
    }
}

#[async_trait::async_trait]
impl super::super::super::engine::TurnObserver for SubagentObserver {
    fn record_usage(
        &mut self,
        provider: &str,
        model: &str,
        usage: &crate::openhuman::inference::provider::UsageInfo,
    ) {
        self.usage.input_tokens += usage.input_tokens;
        self.usage.output_tokens += usage.output_tokens;
        self.usage.cached_input_tokens += usage.cached_input_tokens;
        // Effective per-call cost: backend-charged when present, else the
        // per-model catalog estimate (#4124) — so a sub-agent on a BYO/local
        // provider that bills no charge still contributes a priced cost to the
        // parent turn's net total. `charged_amount_usd` here is the *net cost*,
        // matching the parent adapter's convention.
        let call_cost = crate::openhuman::agent::cost::call_cost_usd(model, usage);
        self.usage.charged_amount_usd += call_cost;
        // Mirror the parent adapter: feed every sub-agent provider call into the
        // global cost tracker so the cost dashboard/summary reflects delegated
        // spend (previously sub-agent tokens were invisible to it). The per-turn
        // rollup into the UI footer happens separately via `turn_subagent_usage`.
        crate::openhuman::cost::record_provider_usage(model, usage);
        // Provenance for the transcript's last assistant message (#4134): keeps
        // the provider + model so per-thread usage reads can price the sub-agent
        // at its actual model.
        self.last_turn_usage = Some(transcript::TurnUsage {
            provider: provider.to_string(),
            model: model.to_string(),
            usage: transcript::MessageUsage {
                input: usage.input_tokens,
                output: usage.output_tokens,
                cached_input: usage.cached_input_tokens,
                context_window: usage.context_window,
                cost_usd: call_cost,
            },
            ts: chrono::Utc::now().to_rfc3339(),
            reasoning_content: None,
            tool_calls: Vec::new(),
            iteration: 0,
        });
    }

    async fn on_assistant(
        &mut self,
        _display_text: &str,
        response_text: &str,
        reasoning_content: Option<&str>,
        native_tool_calls: &[crate::openhuman::inference::provider::ToolCall],
        parsed_calls: &[super::super::super::parse::ParsedToolCall],
        iteration: usize,
        is_final: bool,
    ) {
        let tool_calls = parsed_calls.len();
        if let Some(ref mut usage) = self.last_turn_usage {
            usage.reasoning_content = reasoning_content
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string);
            usage.tool_calls = native_tool_calls.to_vec();
            usage.iteration = (iteration + 1) as u32;
        }
        let extra = if is_final {
            serde_json::json!({
                "scope": "worker_thread",
                "agent_id": self.agent_id,
                "task_id": self.task_id,
                "iteration": iteration + 1,
                "final": true,
            })
        } else {
            serde_json::json!({
                "scope": "worker_thread",
                "agent_id": self.agent_id,
                "task_id": self.task_id,
                "iteration": iteration + 1,
                "tool_calls": tool_calls,
            })
        };
        self.append_worker_message(response_text.to_string(), "agent".to_string(), extra);
    }

    fn on_tool_result(
        &mut self,
        call_id: &str,
        tool_name: &str,
        result_text: &str,
        _success: bool,
        iteration: usize,
    ) {
        // Native mode mirrors each tool result individually; text mode batches
        // them in `on_results_batch` instead.
        if self.force_text_mode {
            return;
        }
        self.append_worker_message(
            result_text.to_string(),
            "user".to_string(),
            serde_json::json!({
                "scope": "worker_thread",
                "agent_id": self.agent_id,
                "task_id": self.task_id,
                "iteration": iteration + 1,
                "tool_call_id": call_id,
                "tool_name": tool_name,
            }),
        );
    }

    fn on_results_batch(&mut self, content: &str, iteration: usize) {
        self.append_worker_message(
            content.to_string(),
            "user".to_string(),
            serde_json::json!({
                "scope": "worker_thread",
                "agent_id": self.agent_id,
                "task_id": self.task_id,
                "iteration": iteration + 1,
                "mode": "text",
            }),
        );
    }

    fn after_iteration(&mut self, history: &[ChatMessage], _iteration: usize) {
        self.persist_transcript(history);
    }
}
