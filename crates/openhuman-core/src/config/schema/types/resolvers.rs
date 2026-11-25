//! Read-only resolvers on [`Config`]: files dir, per-workload
//! local-model routing, and exact agent model pins.

use std::path::PathBuf;

use super::output_language::output_language_directive;
use crate::config::schema::Config;

impl Config {
    /// Locale for host-generated native-channel messages. Unsupported and
    /// unset preferences fall back to English; model-output language and RPC
    /// metadata are not consulted.
    pub fn native_channel_locale(&self) -> &'static str {
        crate::inference::failure_copy::supported_channel_locale(self.channels_locale.as_deref())
            .unwrap_or("en")
    }

    /// Render a classified native-channel failure with this config's trusted
    /// locale preference and the shared product chat-error catalog.
    pub fn render_native_channel_failure(
        &self,
        fallback_message: &str,
        copy_key: Option<&str>,
        copy_params: Option<&serde_json::Value>,
    ) -> String {
        crate::inference::failure_copy::render_channel_failure(
            self.native_channel_locale(),
            fallback_message,
            copy_key,
            copy_params,
        )
    }

    /// The folder agent deliverables are written to (#5505): the Settings
    /// override when set, else `~/OpenHuman/projects/Files`. Every producer
    /// and the boot migration read it through here.
    pub fn files_dir(&self) -> PathBuf {
        crate::config::resolve_files_dir(&self.files_dir_override)
    }

    /// Read the per-workload provider string and return the local model id
    /// when the workload is routed to Ollama.
    ///
    /// Recognised workload names:
    /// `"chat"`, `"reasoning"`, `"agentic"`, `"coding"`, `"vision"`, `"memory"`,
    /// `"embeddings"`.
    ///
    /// Returns `None` when the provider isn't `"ollama:<model>"` (including
    /// when the field is unset, blank, `"cloud"`, or any other prefix).
    /// This is the single source of truth for "is this workload local?" —
    /// callers MUST NOT consult the legacy `local_ai.usage.*` booleans, which are
    /// deprecated zombies kept for migration only.
    pub fn workload_local_model(&self, workload: &str) -> Option<String> {
        let raw = match workload {
            "chat" => self.chat_provider.as_deref(),
            "reasoning" => self.reasoning_provider.as_deref(),
            "agentic" => self.agentic_provider.as_deref(),
            "coding" => self.coding_provider.as_deref(),
            "vision" => self.vision_provider.as_deref(),
            "memory" => self.memory_provider.as_deref(),
            "embeddings" => self.embeddings_provider.as_deref(),
            _ => None,
        }?;
        let trimmed = raw.trim();
        let model = trimmed.strip_prefix("ollama:")?.trim();
        if model.is_empty() {
            None
        } else {
            Some(model.to_string())
        }
    }

    /// `true` when `workload_local_model` returns `Some` for the named
    /// workload. Convenience wrapper for the common "do I dispatch
    /// locally?" branch.
    pub fn workload_uses_local(&self, workload: &str) -> bool {
        self.workload_local_model(workload).is_some()
    }

    /// Prompt directive for background LLM artifacts, if configured.
    pub fn output_language_directive(&self) -> Option<String> {
        output_language_directive(self.output_language.as_deref())
    }

    /// Resolve an exact model pin for an agent, if configured.
    ///
    /// Precedence is intentionally narrow and deterministic:
    /// 1. `orchestrator.model` when resolving the front-line orchestrator.
    /// 2. `[teams.<agent_id>]` entries, with `lead_model` used for agents
    ///    that can delegate and `agent_model` used for leaf workers.
    /// 3. `[teams.<name>]` for an `<name>_agent` id (e.g. `[teams.code]` for
    ///    `code_agent`).
    ///
    /// Empty strings are ignored so partially-written configs fall back to
    /// the existing auto-routing path.
    pub fn configured_agent_model(&self, agent_id: &str, is_team_lead: bool) -> Option<&str> {
        fn clean(model: Option<&str>) -> Option<&str> {
            model.map(str::trim).filter(|value| !value.is_empty())
        }

        // A hosted profile's operator-fixed model outranks every per-agent pin.
        if let Some(model) = self
            .profile_providers
            .as_ref()
            .and_then(|binding| binding.chat_model())
        {
            return Some(model);
        }
        let agent_id = agent_id.trim();
        if agent_id.is_empty() {
            return None;
        }

        if agent_id == "orchestrator" {
            if let Some(model) = clean(self.orchestrator.model.as_deref()) {
                return Some(model);
            }
        }

        if let Some(model) = self
            .teams
            .get(agent_id)
            .and_then(|team| team.model_for_role(is_team_lead))
        {
            return Some(model);
        }

        let stripped = agent_id.strip_suffix("_agent")?;
        self.teams
            .get(stripped)
            .and_then(|team| team.model_for_role(is_team_lead))
    }
}
