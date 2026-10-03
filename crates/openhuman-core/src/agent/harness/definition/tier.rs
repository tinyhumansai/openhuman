//! Spawn-hierarchy tier ([`AgentTier`]) and the single tier-transition rule
//! shared by the loader's static check and the runtime spawn gate.

use serde::{Deserialize, Serialize};

/// Role an agent plays in the spawn hierarchy.
///
/// See [`super::AgentDefinition::agent_tier`] for the full contract. In short:
///
/// ```text
/// Chat (fast, UX-focused)
///   └─► Reasoning (slow, deep-thinking)
///         └─► Worker (leaf executors)
///   └─► Worker (direct fast-path delegation)
/// ```
///
/// `Chat` and `Reasoning` are forbidden from spawning their own tier;
/// `Worker` is forbidden from spawning anything. Total depth is capped
/// at three hops by the harness regardless of tier (defence in depth
/// against custom TOMLs that drop the tier annotation).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentTier {
    /// User-facing fast-tier agent (e.g. the Orchestrator on the
    /// `chat` model hint). Optimised for TTFT, not for long-horizon
    /// reasoning. May delegate to `Reasoning` or `Worker`; must NOT
    /// delegate to another `Chat` agent.
    Chat,
    /// Deep-thinking agent on a `hint:reasoning`-style model (e.g. the
    /// Planner). Decomposes long-running tasks and delegates execution
    /// to one or more `Worker`s. Must NOT delegate to another
    /// `Reasoning` agent.
    Reasoning,
    /// Leaf executor — code executors, critics, summarizers,
    /// integration specialists, etc. Workers do the actual work and must
    /// NOT spawn further subagents (a `Worker` with a non-empty
    /// `subagents` list is rejected by the loader).
    #[default]
    Worker,
}

impl AgentTier {
    /// Human-readable tier name used in error messages.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Reasoning => "reasoning",
            Self::Worker => "worker",
        }
    }
}

impl std::fmt::Display for AgentTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Single source of truth for the spawn-hierarchy rule: is a `parent`-tier
/// agent allowed to delegate to a `child`-tier agent?
///
/// Returns `Ok(())` for the legal handoffs and `Err(reason)` for the three
/// forbidden shapes, where `reason` is a tier-only human-readable explanation
/// (no agent ids — callers prepend their own context):
///
/// - `Worker → *` — workers are leaf executors and must not spawn anything.
/// - `Chat → Chat` — the chat tier is a leaf in its own dimension; cloning it
///   defeats the fast-path and risks unbounded `chat → chat → …` chains.
/// - `Reasoning → Reasoning` — reasoning agents compose downward into workers,
///   not into each other (a depth-blowing recursion of slow models).
///
/// Note this forbids same-tier and worker-as-parent hops, **not** upward hops:
/// `reasoning → chat` is a legal edge (a reasoning agent can hand a follow-up
/// back to the `orchestrator` chat agent), so it must stay allowed. The harness'es `MAX_SPAWN_DEPTH` cap bounds chain length
/// independently of tier direction.
///
/// This is the static authoring rule the loader walks over declared `subagents`
/// pairs at boot (see
/// [`crate::agent::registry::agents::validate_tier_hierarchy`]). The
/// runtime spawn gate (`run_subagent`) reuses it as defense-in-depth, but
/// deliberately exempts worker *parents* — a worker's `subagents` list holds
/// no agent id (the loader rejects one), so the only runtime spawn a worker
/// reaches is one the host dispatched for it, not one it chose.
pub fn validate_tier_transition(parent: AgentTier, child: AgentTier) -> Result<(), String> {
    match (parent, child) {
        (AgentTier::Worker, _) => Err(format!(
            "a `worker` tier agent must not spawn `{}` — workers are leaf executors",
            child.as_str()
        )),
        (AgentTier::Chat, AgentTier::Chat) => Err(
            "the chat tier is a leaf in its own dimension — hand off to a `reasoning` or \
             `worker` agent instead"
                .to_string(),
        ),
        (AgentTier::Reasoning, AgentTier::Reasoning) => {
            Err("reasoning agents compose downward into workers, not into each other".to_string())
        }
        _ => Ok(()),
    }
}
