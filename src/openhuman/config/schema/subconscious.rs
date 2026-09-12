//! Subconscious engine selection.
//!
//! `subconscious.engine` chooses which cognition drives the heartbeat tick's
//! observe/reflect/commit cycle:
//!
//! * `local` (default) — the local tinyagents graph.
//! * `medulla` — **accepted but not implemented in this build.** The
//!   supervised `medulla-serve` child that backed it was removed along with
//!   the `medulla_local` draft; the engine is to be re-ported onto the
//!   `medulla` domain. A config selecting it logs a warning and runs the local
//!   graph.
//!
//! # Why the variant and its settings still exist
//!
//! Deliberate back-compat, not oversight. Configs in the wild may carry
//! `engine = "medulla"` or a `[subconscious.medulla_local]` block, and serde
//! rejects an unknown enum variant — so deleting either would turn a
//! now-unsupported *setting* into a hard **startup failure**. Keeping them as
//! inert serde means such a host boots, runs the local graph, and says why.
//!
//! Remove them only once the engine is re-ported (making the variant live
//! again) or a config migration rewrites the affected keys.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Which engine runs the subconscious reflect/commit cognition.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SubconsciousEngine {
    /// The local tinyagents subconscious graph (unchanged default).
    #[default]
    Local,
    /// Route ticks through the Medulla brain.
    ///
    /// Accepted for back-compat but **not implemented in this build** — see the
    /// module docs. Selecting it warns and falls back to [`Self::Local`].
    Medulla,
}

impl SubconsciousEngine {
    /// Whether the operator selected the medulla brain.
    ///
    /// True does **not** mean ticks route there — the engine is unimplemented
    /// in this build. The tick uses this only to warn before running the local
    /// graph.
    pub fn is_medulla(self) -> bool {
        matches!(self, Self::Medulla)
    }
}

/// Settings for the supervised local `medulla-serve` child.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MedullaLocalConfig {
    /// Path to medulla-v1's built serve entry (`dist/serve/index.js`). Empty
    /// falls back to the `OPENHUMAN_MEDULLA_SERVE_ENTRY` environment override;
    /// with neither set the medulla engine reports its serve entry as
    /// unconfigured (see [`Self::resolved_serve_entry`]).
    #[serde(default)]
    pub serve_entry: String,
    /// Overall deadline, in seconds, for one serve request (from writing the
    /// `req` to receiving its correlated `res`), regardless of interleaved
    /// frame traffic. Distinct from the per-read idle timeout: a child that
    /// keeps streaming frames without ever answering is bounded by this
    /// ceiling. `0` falls back to the default
    /// ([`DEFAULT_REQUEST_DEADLINE_SECS`], 300).
    #[serde(default = "default_request_deadline_secs")]
    pub request_deadline_secs: u64,
}

impl Default for MedullaLocalConfig {
    fn default() -> Self {
        Self {
            serve_entry: String::new(),
            request_deadline_secs: DEFAULT_REQUEST_DEADLINE_SECS,
        }
    }
}

/// Default overall per-request deadline for serve requests, in seconds.
pub const DEFAULT_REQUEST_DEADLINE_SECS: u64 = 300;

/// Ceiling for [`MedullaLocalConfig::request_deadline_secs`]: 24 hours. Larger
/// values add no practical headroom and a near-`u64::MAX` duration would panic
/// in `Instant + Duration` arithmetic on the request path.
pub const MAX_REQUEST_DEADLINE_SECS: u64 = 24 * 60 * 60;

fn default_request_deadline_secs() -> u64 {
    DEFAULT_REQUEST_DEADLINE_SECS
}

/// Environment override for the serve entry when `serve_entry` is left unset.
///
/// There is no portable compiled-in default: medulla-v1's built `dist/serve`
/// lives outside this repo, so its location is deployment-specific. A developer
/// pointing at their umbrella checkout sets this env var (or the config field)
/// rather than relying on a machine-local path baked into the binary.
const SERVE_ENTRY_ENV: &str = "OPENHUMAN_MEDULLA_SERVE_ENTRY";

impl MedullaLocalConfig {
    /// The resolved serve entry, or `None` when it is unconfigured.
    ///
    /// Precedence: the explicit `serve_entry` config value, then the
    /// `OPENHUMAN_MEDULLA_SERVE_ENTRY` environment override. When neither is
    /// set this returns `None` — the medulla engine then reports the serve
    /// entry as unconfigured instead of pointing at a machine-local path.
    pub fn resolved_serve_entry(&self) -> Option<std::path::PathBuf> {
        Self::resolve_entry(&self.serve_entry, std::env::var(SERVE_ENTRY_ENV).ok())
    }

    /// The effective overall per-request deadline. A configured `0` (an
    /// explicit zero would disable the ceiling entirely — never wanted) falls
    /// back to the default.
    pub fn request_deadline(&self) -> std::time::Duration {
        let secs = if self.request_deadline_secs == 0 {
            tracing::warn!(
                configured = self.request_deadline_secs,
                effective_secs = DEFAULT_REQUEST_DEADLINE_SECS,
                "medulla_local request_deadline_secs=0 would disable the request ceiling; using the default"
            );
            DEFAULT_REQUEST_DEADLINE_SECS
        } else if self.request_deadline_secs > MAX_REQUEST_DEADLINE_SECS {
            tracing::warn!(
                configured = self.request_deadline_secs,
                effective_secs = MAX_REQUEST_DEADLINE_SECS,
                "medulla_local request_deadline_secs exceeds the 24h ceiling; clamping"
            );
            MAX_REQUEST_DEADLINE_SECS
        } else {
            self.request_deadline_secs
        };
        std::time::Duration::from_secs(secs)
    }

    /// Pure resolver shared by [`Self::resolved_serve_entry`], factored out so
    /// the precedence rules are testable without mutating process env.
    fn resolve_entry(configured: &str, env_override: Option<String>) -> Option<std::path::PathBuf> {
        let trimmed = configured.trim();
        if !trimmed.is_empty() {
            return Some(std::path::PathBuf::from(trimmed));
        }
        let env = env_override?;
        let env = env.trim();
        if env.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(env))
        }
    }
}

/// The `[subconscious]` config block.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SubconsciousConfig {
    /// Which engine drives the subconscious tick. Default `local`.
    #[serde(default)]
    pub engine: SubconsciousEngine,
    /// Local `medulla-serve` child settings (only used when `engine = medulla`).
    #[serde(default)]
    pub medulla_local: MedullaLocalConfig,
}

#[cfg(test)]
#[path = "subconscious_tests.rs"]
mod tests;
