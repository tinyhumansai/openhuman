//! Config sub-facade — the first typed surface, and the proof of the pattern.
//!
//! Every other sub-facade (memory, workflows, chat, medulla, …) follows the
//! shape established here:
//!
//! 1. A borrowed newtype over `&Arc<CoreRuntime>` — zero-cost, no state.
//! 2. Facade-owned serde types, so hosts never name a domain's internal type
//!    and never touch `serde_json::Value`.
//! 3. Two-line methods delegating to [`call`](super::call::call).
//!
//! Config is deliberately first because it is registered under
//! `DomainGroup::Platform`, which every preset enables — so a failure here is
//! unambiguously a facade bug rather than a gating question.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::call::call;
use super::error::CoreError;
use crate::core::runtime::CoreRuntime;

/// Environment-driven runtime flags.
///
/// Mirrors the core's `RuntimeFlagsOut` (`config/ops/loader.rs`). Declared here
/// rather than re-exported so the facade's wire contract is explicit and a
/// change to the core's internal struct surfaces as a failing round-trip test
/// instead of silently altering the host-facing API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeFlags {
    /// `OPENHUMAN_BROWSER_ALLOW_ALL` — browser tools bypass the allowlist.
    pub browser_allow_all: bool,
    /// `OPENHUMAN_LOG_PROMPTS` — full prompts are written to the log.
    pub log_prompts: bool,
}

/// Typed access to persisted and environment configuration.
///
/// Obtained from [`Core::config`](super::Core::config); never constructed
/// directly.
pub struct Config<'a>(pub(super) &'a Arc<CoreRuntime>);

impl Config<'_> {
    /// Read the environment-driven runtime flags.
    ///
    /// Note this method always travels wrapped in the `{"result", "logs"}`
    /// envelope, because its handler emits a log unconditionally. That is
    /// handled in [`call`](super::call::call) and is invisible here — which is
    /// the entire point of routing every method through one helper.
    pub async fn runtime_flags(&self) -> Result<RuntimeFlags, CoreError> {
        call(
            self.0,
            "openhuman.config_get_runtime_flags",
            serde_json::json!({}),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn runtime_flags_decodes_the_core_wire_shape() {
        // Exactly what `RuntimeFlagsOut` serializes to. If the core struct
        // gains or renames a field, this fails and the facade type gets
        // updated deliberately rather than drifting.
        let wire = json!({ "browser_allow_all": true, "log_prompts": false });
        let flags: RuntimeFlags = serde_json::from_value(wire).expect("decodes");
        assert!(flags.browser_allow_all);
        assert!(!flags.log_prompts);
    }

    #[test]
    fn runtime_flags_round_trips() {
        let original = RuntimeFlags {
            browser_allow_all: false,
            log_prompts: true,
        };
        let encoded = serde_json::to_value(&original).expect("encodes");
        let decoded: RuntimeFlags = serde_json::from_value(encoded).expect("decodes");
        assert_eq!(decoded, original);
    }

    #[test]
    fn runtime_flags_rejects_a_missing_field() {
        // No `#[serde(default)]`: a shape change must fail loudly at the
        // facade boundary rather than silently defaulting to `false`, which
        // would read as "the flag is off" instead of "we lost the field".
        let wire = json!({ "browser_allow_all": true });
        assert!(serde_json::from_value::<RuntimeFlags>(wire).is_err());
    }
}
