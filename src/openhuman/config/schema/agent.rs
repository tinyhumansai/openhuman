//! Agent and delegate agent configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Optional model pin for the front-line orchestrator.
///
/// This is intentionally a small exact-model override: provider routing
/// still comes from the normal reasoning workload, and this field only
/// replaces the final model id when present.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OrchestratorModelConfig {
    pub model: Option<String>,
}

/// Optional per-team model pins used by delegation.
///
/// `lead_model` applies to agents that themselves expose sub-agents;
/// `agent_model` applies to leaf workers. Callers fall back across the
/// pair so configs can specify only one tier without breaking routing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TeamModelConfig {
    pub lead_model: Option<String>,
    pub agent_model: Option<String>,
}

impl TeamModelConfig {
    pub fn model_for_role(&self, is_team_lead: bool) -> Option<&str> {
        let lead_model = self
            .lead_model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty());
        let agent_model = self
            .agent_model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty());

        if is_team_lead {
            lead_model.or(agent_model)
        } else {
            agent_model.or(lead_model)
        }
    }
}

/// User-facing memory-context window preset.
///
/// Each preset maps deterministically (via [`MemoryContextWindow::limits`])
/// to the actual character budgets used by the agent harness when
/// injecting recalled memory and the long-term memory summary tree into
/// new agent / orchestrator sessions. The mapping is the single source
/// of truth — the frontend never decides budgets directly. Presets are
/// bounded (`Maximum` ≈ 8 000 chars of recall + ≈ 128 000 chars of root
/// summary, ≈ 32k tokens) so users cannot accidentally blow up prompts.
///
/// See `gitbooks/developing/memory-context-window.md` for the user-facing tradeoff
/// guidance and the per-preset numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum MemoryContextWindow {
    /// Cheapest, lightest. Tight recall + tree-summary budget.
    Minimal,
    /// Sensible default — current behaviour.
    #[default]
    Balanced,
    /// More continuity at the cost of more tokens per run.
    Extended,
    /// Maximum allowed continuity — meaningfully larger token bill.
    Maximum,
}

/// Concrete character budgets resolved from a [`MemoryContextWindow`]
/// preset. All three caps are bounded to keep prompt growth safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryWindowLimits {
    /// Cap for `[Memory context]` + `[User working memory]` injection
    /// produced by `DefaultMemoryLoader`.
    pub max_memory_context_chars: usize,
    /// Per-namespace cap when collecting tree-summarizer root summaries
    /// for the system prompt (first turn only).
    pub per_namespace_max_chars: usize,
    /// Hard ceiling across all namespaces for the tree-summary block.
    pub total_tree_max_chars: usize,
}

impl MemoryContextWindow {
    /// Return the canonical budgets for this preset. The mapping is
    /// intentionally stepped (no continuous slider) so the UI and core
    /// stay aligned and impact is predictable.
    pub fn limits(self) -> MemoryWindowLimits {
        match self {
            MemoryContextWindow::Minimal => MemoryWindowLimits {
                max_memory_context_chars: 800,
                per_namespace_max_chars: 2_000,
                total_tree_max_chars: 8_000,
            },
            MemoryContextWindow::Balanced => MemoryWindowLimits {
                max_memory_context_chars: 2_000,
                per_namespace_max_chars: 8_000,
                total_tree_max_chars: 32_000,
            },
            MemoryContextWindow::Extended => MemoryWindowLimits {
                max_memory_context_chars: 4_000,
                per_namespace_max_chars: 16_000,
                total_tree_max_chars: 64_000,
            },
            MemoryContextWindow::Maximum => MemoryWindowLimits {
                max_memory_context_chars: 8_000,
                per_namespace_max_chars: 32_000,
                total_tree_max_chars: 128_000,
            },
        }
    }

    /// Stable lowercase label for serialization across CLI / RPC / UI.
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryContextWindow::Minimal => "minimal",
            MemoryContextWindow::Balanced => "balanced",
            MemoryContextWindow::Extended => "extended",
            MemoryContextWindow::Maximum => "maximum",
        }
    }

    /// Parse from the lowercase label produced by [`Self::as_str`].
    /// Returns `None` for unknown inputs so callers can fall back.
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "minimal" => Some(Self::Minimal),
            "balanced" => Some(Self::Balanced),
            "extended" => Some(Self::Extended),
            "maximum" => Some(Self::Maximum),
            _ => None,
        }
    }
}

/// Configuration for a delegate sub-agent used by the `delegate` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DelegateAgentConfig {
    /// Model name (inference uses the OpenHuman backend from main config).
    pub model: String,
    /// Optional system prompt for the sub-agent
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Temperature override
    #[serde(default)]
    pub temperature: Option<f64>,
    /// Max recursion depth for nested delegation
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
}

fn default_max_depth() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AgentConfig {
    /// When true: bootstrap_max_chars=6000, rag_chunk_limit=2. Use for 13B or smaller models.
    #[serde(default)]
    pub compact_context: bool,
    #[serde(default = "default_agent_max_tool_iterations")]
    pub max_tool_iterations: usize,
    #[serde(default = "default_agent_max_history_messages")]
    pub max_history_messages: usize,
    #[serde(default)]
    pub parallel_tools: bool,
    /// Maximum number of tool calls to execute concurrently when `parallel_tools` is true.
    #[serde(default = "default_max_parallel_tools")]
    pub max_parallel_tools: usize,
    #[serde(default = "default_agent_tool_dispatcher")]
    pub tool_dispatcher: String,
    /// **Legacy** — maximum characters of memory context to inject per
    /// turn. Prefer [`AgentConfig::memory_window`]; this field is only
    /// honoured for unmigrated configs (those that have never set the
    /// preset). Once a preset is explicitly chosen, the preset is
    /// authoritative and this value is ignored.
    #[serde(default = "default_max_memory_context_chars")]
    pub max_memory_context_chars: usize,
    /// Stepped user-facing preset that maps to the actual memory
    /// injection budgets. See [`MemoryContextWindow`].
    ///
    /// `None` means "no preset has been chosen yet" (e.g. a config
    /// upgraded from a build that predates this setting). In that
    /// case [`AgentConfig::resolved_memory_limits`] honours the legacy
    /// raw `max_memory_context_chars` field for backward compatibility.
    /// Once the user picks a preset (or any caller writes one) it
    /// becomes authoritative — the raw field is then ignored, so the
    /// UI control is the single source of truth from that point on.
    #[serde(default)]
    pub memory_window: Option<MemoryContextWindow>,
    /// Per-channel maximum permission level for tool execution.
    /// Keys are channel names (e.g., "telegram", "discord", "web", "cli").
    /// Values are permission levels: "none", "readonly" (or "read_only"),
    /// "write", "execute", "dangerous".
    ///
    /// Runtime semantics (see
    /// [`crate::openhuman::agent_tool_policy::engine::ToolPolicyEngine`]):
    ///
    /// * **Empty map** — the policy engine preserves the legacy
    ///   unrestricted surface and returns `PermissionLevel::Dangerous`
    ///   for every channel. This branch only matters before the
    ///   one-time migration runs.
    /// * **Non-empty map, channel present** — the configured level is
    ///   used.
    /// * **Non-empty map, channel absent** — the engine falls back to
    ///   `PermissionLevel::ReadOnly` (the fail-closed default for an
    ///   already-policy-managed install).
    ///
    /// [`AgentConfig::migrate_channel_permissions_if_legacy`] seeds the
    /// map with `web=Execute` + each configured channel = `Execute` on
    /// first boot after upgrade, so legacy installs land in the
    /// non-empty branch before any tool dispatch happens. New installs
    /// ship with an explicit map. The empty-map "Dangerous" branch is
    /// effectively reachable only by an operator manually wiping the
    /// map in their on-disk config; if you change that branch's
    /// behavior, update `AGENTS.md` and the engine docstring in lock-step.
    #[serde(default)]
    pub channel_permissions: std::collections::HashMap<String, String>,

    /// Maximum byte length of a single tool-result body before the
    /// context pipeline's tool-result budget stage truncates it. Applied
    /// inline at tool-execution time (before the result enters history),
    /// so it is cache-safe. `0` disables the cap. Defaults to
    /// `DEFAULT_TOOL_RESULT_BUDGET_BYTES` (16 KiB).
    #[serde(default = "default_tool_result_budget_bytes")]
    pub tool_result_budget_bytes: usize,

    /// Wall-clock timeout, in seconds, for a single tool/action execution
    /// (and the per-agent delegated chat call). Bounded to
    /// `tool_timeout::MIN_TIMEOUT_SECS..=tool_timeout::MAX_TIMEOUT_SECS`
    /// (`1..=3600`); the default is `tool_timeout::DEFAULT_TIMEOUT_SECS`
    /// (120). Surfaced in **Settings → Agent OS access → Action timeout** so
    /// users running large local models can extend it without editing config
    /// files (issue #3100). Pushed into the live
    /// [`crate::openhuman::tool_timeout`] runtime on save; the
    /// `OPENHUMAN_TOOL_TIMEOUT_SECS` env var still overrides it when set.
    #[serde(default = "default_agent_timeout_secs")]
    pub agent_timeout_secs: u64,
}

fn default_tool_result_budget_bytes() -> usize {
    crate::openhuman::context::DEFAULT_TOOL_RESULT_BUDGET_BYTES
}

fn default_agent_timeout_secs() -> u64 {
    crate::openhuman::tool_timeout::DEFAULT_TIMEOUT_SECS
}

fn default_agent_max_tool_iterations() -> usize {
    10
}

fn default_agent_max_history_messages() -> usize {
    50
}

fn default_max_parallel_tools() -> usize {
    4
}

fn default_agent_tool_dispatcher() -> String {
    "auto".into()
}

fn default_max_memory_context_chars() -> usize {
    2000
}

impl AgentConfig {
    /// Seed legacy installs whose channel-permissions map is empty and
    /// that already have at least one non-web channel configured,
    /// writing explicit per-channel execute entries.
    ///
    /// The engine layer keeps its legacy empty-map shortcut; this
    /// migration replaces it with an explicit policy so the
    /// per-channel cap engages on the very first boot after upgrade.
    /// `known_channels` is the set of channels the user has configured
    /// in `channels::ChannelsConfig`. The web channel is always added
    /// on top so the desktop UI stays usable.
    ///
    /// Returns `true` when a migration write is required so the caller
    /// can save and reload; returns `false` when the map was already
    /// populated, no non-web channels were configured (fresh install,
    /// engine's legacy unrestricted shortcut continues), or the
    /// migration is otherwise a no-op. Idempotent.
    pub fn migrate_channel_permissions_if_legacy<I, S>(&mut self, known_channels: I) -> bool
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        if !self.channel_permissions.is_empty() {
            return false;
        }
        let extra: Vec<String> = known_channels
            .into_iter()
            .map(|s| s.as_ref().to_string())
            .filter(|s| !s.is_empty() && s != "web")
            .collect();
        if extra.is_empty() {
            // No channels configured yet — leave the map empty so the
            // engine's legacy "empty == unrestricted" shortcut keeps
            // ruling fresh installs the same way it did pre-PR. The
            // migration is for installs that already have channels
            // active under the legacy unrestricted surface.
            return false;
        }
        // Seed web + every known channel = execute so the engine's
        // per-channel cap evaluates against an explicit policy instead
        // of the legacy unrestricted default.
        let names: Vec<String> = std::iter::once("web".to_string()).chain(extra).collect();
        for name in &names {
            self.channel_permissions
                .insert(name.clone(), "execute".to_string());
        }
        log::info!(
            target: "openhuman::config",
            "[agent-config] channel_permissions: migrated {} legacy channels to execute (preserved pre-PR behavior): {:?}",
            names.len(),
            names
        );
        true
    }

    /// Resolve the active memory-context budgets for this agent config.
    ///
    /// Two cases:
    ///
    /// 1. **Preset chosen** (`memory_window = Some(_)`) — the preset is
    ///    authoritative. The legacy raw `max_memory_context_chars`
    ///    field is ignored entirely. This is the steady-state path: the
    ///    UI control is the single source of truth.
    ///
    /// 2. **Unmigrated config** (`memory_window = None`) — fall back to
    ///    the legacy raw `max_memory_context_chars` for the recall cap
    ///    so a config upgraded from an older build keeps its previous
    ///    recall behaviour. The raw value is still bounded by the
    ///    `Maximum` preset's recall cap so safety limits are preserved.
    ///    Tree-summary caps come from the `Balanced` baseline because
    ///    older builds had no notion of a per-namespace tree cap on
    ///    this code path.
    pub fn resolved_memory_limits(&self) -> MemoryWindowLimits {
        match self.memory_window {
            Some(window) => window.limits(),
            None => {
                let mut limits = MemoryContextWindow::Balanced.limits();
                let hard_cap = MemoryContextWindow::Maximum
                    .limits()
                    .max_memory_context_chars;
                limits.max_memory_context_chars = self.max_memory_context_chars.min(hard_cap);
                limits
            }
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            compact_context: false,
            max_tool_iterations: default_agent_max_tool_iterations(),
            max_history_messages: default_agent_max_history_messages(),
            parallel_tools: false,
            max_parallel_tools: default_max_parallel_tools(),
            tool_dispatcher: default_agent_tool_dispatcher(),
            max_memory_context_chars: default_max_memory_context_chars(),
            memory_window: None,
            channel_permissions: std::collections::HashMap::new(),
            tool_result_budget_bytes: default_tool_result_budget_bytes(),
            agent_timeout_secs: default_agent_timeout_secs(),
        }
    }
}

#[cfg(test)]
mod memory_window_tests {
    use super::*;

    #[test]
    fn presets_are_strictly_ordered_and_bounded() {
        let m = MemoryContextWindow::Minimal.limits();
        let b = MemoryContextWindow::Balanced.limits();
        let e = MemoryContextWindow::Extended.limits();
        let max = MemoryContextWindow::Maximum.limits();

        // Recall cap grows monotonically with preset size.
        assert!(m.max_memory_context_chars < b.max_memory_context_chars);
        assert!(b.max_memory_context_chars < e.max_memory_context_chars);
        assert!(e.max_memory_context_chars < max.max_memory_context_chars);

        // Tree summary caps grow monotonically too.
        assert!(m.per_namespace_max_chars < b.per_namespace_max_chars);
        assert!(b.per_namespace_max_chars < e.per_namespace_max_chars);
        assert!(e.per_namespace_max_chars < max.per_namespace_max_chars);
        assert!(m.total_tree_max_chars < max.total_tree_max_chars);

        // Hard ceiling is bounded — Maximum still leaves headroom in a
        // typical 200k-token context window.
        assert!(max.total_tree_max_chars <= 128_000);
    }

    #[test]
    fn balanced_matches_legacy_defaults() {
        // Balanced preset must keep historical behaviour: 2 000 char
        // recall budget and 32 000 char total tree-summary cap (used to
        // be hard-coded constants in `agent/prompts/types.rs`).
        let b = MemoryContextWindow::Balanced.limits();
        assert_eq!(b.max_memory_context_chars, 2_000);
        assert_eq!(b.per_namespace_max_chars, 8_000);
        assert_eq!(b.total_tree_max_chars, 32_000);
    }

    #[test]
    fn default_agent_config_is_unmigrated_and_resolves_to_balanced_caps() {
        // Default = `memory_window: None` (unmigrated). The recall cap
        // falls back to the legacy `max_memory_context_chars` default
        // (2 000), which matches Balanced — so the resolved limits are
        // byte-identical to the historical behaviour.
        let cfg = AgentConfig::default();
        assert_eq!(cfg.memory_window, None);
        assert_eq!(
            cfg.resolved_memory_limits(),
            MemoryContextWindow::Balanced.limits()
        );
    }

    #[test]
    fn explicit_preset_is_authoritative_and_ignores_legacy_raw_field() {
        // Once Minimal is chosen, the preset's recall cap (800) is what
        // the harness sees — even if the legacy raw field still holds a
        // wider value from before the user picked a preset. Without
        // this, switching to `Minimal` in the UI would silently fail to
        // shrink the recall budget.
        let cfg = AgentConfig {
            memory_window: Some(MemoryContextWindow::Minimal),
            max_memory_context_chars: 4_000,
            ..AgentConfig::default()
        };
        assert_eq!(
            cfg.resolved_memory_limits(),
            MemoryContextWindow::Minimal.limits(),
            "explicit preset must override legacy raw field"
        );
    }

    #[test]
    fn unmigrated_config_honours_legacy_raw_field_within_safety_ceiling() {
        // Unmigrated power-user config with a legacy override of 4 000
        // keeps that recall cap on upgrade so behaviour doesn't shrink
        // silently. Tree caps come from the Balanced baseline because
        // older builds had no per-namespace cap on this code path.
        let cfg = AgentConfig {
            memory_window: None,
            max_memory_context_chars: 4_000,
            ..AgentConfig::default()
        };
        let limits = cfg.resolved_memory_limits();
        assert_eq!(limits.max_memory_context_chars, 4_000);
        assert_eq!(
            limits.per_namespace_max_chars,
            MemoryContextWindow::Balanced
                .limits()
                .per_namespace_max_chars
        );

        // An unbounded legacy value is clamped to the Maximum preset's
        // recall cap so on-disk overrides can't blow up prompts.
        let runaway = AgentConfig {
            memory_window: None,
            max_memory_context_chars: 1_000_000,
            ..AgentConfig::default()
        };
        assert_eq!(
            runaway.resolved_memory_limits().max_memory_context_chars,
            MemoryContextWindow::Maximum
                .limits()
                .max_memory_context_chars
        );
    }

    #[test]
    fn switching_preset_can_shrink_recall_below_legacy_value() {
        // Regression for the CodeRabbit concern: an unmigrated config
        // with a wide legacy override that then explicitly picks
        // `Minimal` in the UI must end up with the Minimal recall cap,
        // not the legacy value.
        let mut cfg = AgentConfig {
            memory_window: None,
            max_memory_context_chars: 4_000,
            ..AgentConfig::default()
        };
        assert_eq!(cfg.resolved_memory_limits().max_memory_context_chars, 4_000);
        cfg.memory_window = Some(MemoryContextWindow::Minimal);
        assert_eq!(
            cfg.resolved_memory_limits().max_memory_context_chars,
            MemoryContextWindow::Minimal
                .limits()
                .max_memory_context_chars
        );
    }

    #[test]
    fn from_str_opt_round_trips() {
        for window in [
            MemoryContextWindow::Minimal,
            MemoryContextWindow::Balanced,
            MemoryContextWindow::Extended,
            MemoryContextWindow::Maximum,
        ] {
            assert_eq!(
                MemoryContextWindow::from_str_opt(window.as_str()),
                Some(window)
            );
        }
        assert_eq!(
            MemoryContextWindow::from_str_opt("MAXIMUM"),
            Some(MemoryContextWindow::Maximum)
        );
        assert_eq!(MemoryContextWindow::from_str_opt("nonsense"), None);
    }

    #[test]
    fn enum_serializes_as_lowercase_string() {
        let json = serde_json::to_string(&MemoryContextWindow::Extended).unwrap();
        assert_eq!(json, "\"extended\"");
        let back: MemoryContextWindow = serde_json::from_str("\"minimal\"").unwrap();
        assert_eq!(back, MemoryContextWindow::Minimal);
    }

    #[test]
    fn empty_channel_permissions_with_existing_channels_migrates_to_execute() {
        // Legacy install: channel_permissions empty but the user has two
        // channels configured. The migration seeds web + each existing
        // channel = execute so the new fail-closed default doesn't
        // regress them.
        let mut cfg = AgentConfig::default();
        assert!(cfg.channel_permissions.is_empty());

        let known = vec!["telegram".to_string(), "discord".to_string()];
        let migrated = cfg.migrate_channel_permissions_if_legacy(known.iter());

        assert!(migrated, "legacy install must migrate");
        assert_eq!(cfg.channel_permissions.len(), 3);
        for ch in ["web", "telegram", "discord"] {
            assert_eq!(
                cfg.channel_permissions.get(ch).map(String::as_str),
                Some("execute"),
                "expected execute for channel {ch}"
            );
        }
    }

    #[test]
    fn migrate_channel_permissions_idempotent() {
        let mut cfg = AgentConfig::default();
        cfg.migrate_channel_permissions_if_legacy(vec!["telegram".to_string()].iter());
        let again = cfg.migrate_channel_permissions_if_legacy(vec!["telegram".to_string()].iter());
        assert!(!again, "second migration call must be a no-op");
    }

    #[test]
    fn migrate_channel_permissions_with_no_channels_is_noop() {
        // Fresh install with no configured channels and an empty map —
        // no migration needed (the engine fails closed on lookup).
        let mut cfg = AgentConfig::default();
        let migrated = cfg.migrate_channel_permissions_if_legacy(Vec::<String>::new());
        assert!(!migrated);
        assert!(cfg.channel_permissions.is_empty());
    }
}
