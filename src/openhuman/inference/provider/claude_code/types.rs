//! Shared types for the Claude Code CLI provider.

use serde::{Deserialize, Serialize};

/// Minimum supported `claude` CLI version. Below this, the provider refuses
/// to start so we never feed an unsupported stream-json schema into the
/// parser.
pub const MIN_CLI_VERSION: &str = "2.0.0";

/// Outcome of probing the `claude` CLI binary on disk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CliStatus {
    Ok {
        version: String,
        path: String,
    },
    NotInstalled,
    Outdated {
        version: String,
        min_required: String,
        path: String,
    },
    Unusable {
        path: String,
        reason: String,
    },
}

/// Branding string used in user-facing copy. Locked decision (PLAN §13.4).
pub const BRAND_LABEL: &str = "Claude Code CLI";
