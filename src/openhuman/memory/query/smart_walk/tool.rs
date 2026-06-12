//! `SmartMemoryWalkTool` — the agent-facing tool wrapper, plus the
//! `ChatProviderAdapter` that bridges the memory chat provider to the
//! inference `Provider` trait.

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::inference::provider::traits::{ChatMessage, Provider};
use crate::openhuman::memory::chat::{build_chat_provider, ChatPrompt};
use crate::openhuman::memory::query::smart_walk::runner::run_smart_walk;
use crate::openhuman::memory::query::smart_walk::types::{
    truncate_chars, SmartWalkOptions, HARD_MAX_TURNS,
};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult};
use async_trait::async_trait;
use serde_json::json;

// ── Tool ─────────────────────────────────────────────────────────────────────

pub struct SmartMemoryWalkTool;

#[async_trait]
impl Tool for SmartMemoryWalkTool {
    fn name(&self) -> &str {
        "memory_smart_walk"
    }

    fn description(&self) -> &str {
        "Smart memory retrieval — combines vector search, keyword search, \
         entity lookup, and tree browsing to answer queries about the user's \
         memory. More capable than the basic walk: searches across raw files, \
         wiki summaries, documents, and episodic memories."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural-language question to answer by searching memory."
                },
                "namespace": {
                    "type": "string",
                    "description": "Memory namespace. Default: \"default\"."
                },
                "max_turns": {
                    "type": "integer",
                    "description": "Max LLM turns. Default 12, hard cap 25."
                },
                "model": {
                    "type": "string",
                    "description": "Provider:model override (e.g. 'deepseek:deepseek-chat')."
                }
            },
            "required": ["query"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("memory_smart_walk: `query` is required"))?
            .to_string();

        let namespace = args
            .get("namespace")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string();

        let max_turns = args
            .get("max_turns")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).min(HARD_MAX_TURNS))
            .unwrap_or(12);

        let model = args
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let cfg = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("memory_smart_walk: load config failed: {e}"))?;

        let opts = SmartWalkOptions {
            max_turns,
            namespace,
            model,
            content_root: None,
        };

        let chat_provider = build_chat_provider(&cfg)
            .map_err(|e| anyhow::anyhow!("memory_smart_walk: build chat provider failed: {e}"))?;
        let adapter = ChatProviderAdapter {
            inner: chat_provider,
        };

        let outcome = run_smart_walk(&cfg, &adapter, &query, opts).await?;

        let mut out = format!("{}\n", outcome.answer);

        if !outcome.evidence.is_empty() {
            out.push_str("\n## Evidence\n");
            for (i, ev) in outcome.evidence.iter().enumerate() {
                out.push_str(&format!(
                    "{}. **{}** — {}\n   > {}\n",
                    i + 1,
                    ev.source_path,
                    ev.relevance,
                    truncate_chars(&ev.snippet, 200)
                ));
            }
        }

        out.push_str("\n## Trace\n");
        for step in &outcome.trace {
            out.push_str(&format!(
                "- **Turn {}** `{}` {}: {}\n",
                step.turn, step.action, step.args_summary, step.result_preview
            ));
        }
        out.push_str(&format!(
            "\n*Stop reason: {:?}, turns used: {}*\n",
            outcome.stopped_reason, outcome.turns_used
        ));

        Ok(ToolResult::success(out))
    }
}

// ── ChatProviderAdapter ───────────────────────────────────────────────────────

pub(crate) struct ChatProviderAdapter {
    pub(crate) inner: std::sync::Arc<dyn crate::openhuman::memory::chat::ChatProvider>,
}

#[async_trait]
impl Provider for ChatProviderAdapter {
    async fn chat_with_system(
        &self,
        system: Option<&str>,
        message: &str,
        _model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let prompt = ChatPrompt {
            system: system.unwrap_or("").to_string(),
            user: message.to_string(),
            temperature,
            kind: "memory_smart_walk",
        };
        self.inner.chat_for_text(&prompt).await
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let system = messages
            .iter()
            .find(|m| m.role == "system")
            .map(|m| m.content.as_str());
        let user: String = messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        self.chat_with_system(system, &user, model, temperature)
            .await
    }
}
