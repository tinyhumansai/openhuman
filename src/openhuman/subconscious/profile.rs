//! The `SubconsciousProfile` contract — one "world" a subconscious can be
//! instantiated over.
//!
//! A profile is the injected runtime for the generic [`super::instance`] tick
//! graph: its methods are the graph's nodes (observe → prepare_context →
//! reflect → commit). The runner owns everything world-agnostic (tick lock,
//! generation/supersede, timeout, provider gate + rate-cap halt, status); the
//! profile owns only what differs between worlds. Adding a new world is a new
//! profile file, not another engine.
//!
//! Two profiles ship today (phases 2–3):
//! - `memory` — the user's connected sources; observes a memory_diff, reflects
//!   with the slim decision agent (to-dos, goals, `notify_user`, delegation).
//! - `tinyplace` — the tiny.place orchestration world; observes the compressed
//!   execution history + world-diff, reflects with a tool-free steering
//!   synthesis that emits `STEERING_DIRECTIVE`s.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::openhuman::agent::turn_origin::TrustedAutomationSource;
use crate::openhuman::config::Config;

/// What a profile's `observe` stage found changed in its world since the last
/// baseline. Serde so it can ride in the tick graph's checkpointed state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    /// Rendered world diff handed to `reflect` (empty when quiet).
    pub rendered: String,
    /// Whether this window is worth a reflection turn. `false` short-circuits
    /// the runner's quiet path (no LLM call — quiet ticks cost nothing).
    pub has_changes: bool,
    /// Whether the change window contains third-party content — the taint input
    /// for [`SubconsciousProfile::origin`].
    pub has_external_content: bool,
    /// Opaque token `commit` uses to advance exactly the observed window
    /// (memory: none — it re-checkpoints the whole world; tinyplace: the newest
    /// reviewed compressed-row `created_at` for the review cursor).
    pub commit_token: Option<String>,
}

/// The outcome of a profile's `reflect` stage. Serde so it can ride in the tick
/// graph's checkpointed state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reflection {
    /// The decision agent acted through tools (memory profile).
    Acted { response_chars: usize },
    /// A steering directive was emitted (tinyplace profile).
    Steered { directive_id: i64 },
    /// The model looked and correctly chose to do nothing.
    Idle,
}

/// One "world" a subconscious can be instantiated over. See the module docs.
#[async_trait::async_trait]
pub trait SubconsciousProfile: Send + Sync {
    /// Stable instance id — store-key namespace, log prefix, RPC name.
    /// (`"memory"` | `"tinyplace"`.)
    fn id(&self) -> &'static str;

    /// Tick cadence for this world (the heartbeat fans out on this).
    fn cadence(&self, config: &Config) -> Duration;

    /// Stage 1: what changed in this world since my baseline?
    ///
    /// Infallible by signature: errors and first-ever ticks surface as an empty
    /// observation (`has_changes == false`) so the runner's branch-free quiet
    /// path handles them uniformly. The profile logs its own errors.
    async fn observe(&self, config: &Config) -> Observation;

    /// Stage 2 (optional): grounding context for the reflection turn. The
    /// default returns `""` (the tinyplace profile skips this — steering is
    /// deliberately tool-free). Runs only when `obs.has_changes`.
    async fn prepare_context(&self, _config: &Config, _obs: &Observation) -> String {
        String::new()
    }

    /// Stage 3: the reflection turn. Runs only when `obs.has_changes`.
    ///
    /// Returns `Err(String)` on a genuine failure (agent/provider error) so the
    /// runner can classify it (tool-capability, permanent rate cap) and hold the
    /// baseline; a "looked and did nothing" result is `Ok(Reflection::Idle)`.
    async fn reflect(
        &self,
        config: &Config,
        obs: &Observation,
        prepared_context: &str,
    ) -> Result<Reflection, String>;

    /// Advance this world's baseline/cursor. Called by the runner after a quiet
    /// tick (refresh the baseline) and after a successful reflect — never after
    /// a failed or superseded one.
    async fn commit(&self, config: &Config, obs: &Observation);

    /// Turn-origin taint for this observation (memory: tainted iff the diff
    /// carried external content; tinyplace: always tainted).
    fn origin(&self, obs: &Observation) -> TrustedAutomationSource;
}
