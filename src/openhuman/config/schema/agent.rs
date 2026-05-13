//! Agent and delegate agent configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
    /// Values are permission levels: "none", "readonly", "write", "execute", "dangerous".
    /// Channels not listed default to "readonly".
    #[serde(default)]
    pub channel_permissions: std::collections::HashMap<String, String>,

    /// Maximum byte length of a single tool-result body before the
    /// context pipeline's tool-result budget stage truncates it. Applied
    /// inline at tool-execution time (before the result enters history),
    /// so it is cache-safe. `0` disables the cap. Defaults to
    /// `DEFAULT_TOOL_RESULT_BUDGET_BYTES` (16 KiB).
    #[serde(default = "default_tool_result_budget_bytes")]
    pub tool_result_budget_bytes: usize,
}

fn default_tool_result_budget_bytes() -> usize {
    crate::openhuman::context::DEFAULT_TOOL_RESULT_BUDGET_BYTES
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
}
