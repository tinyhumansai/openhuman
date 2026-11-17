//! OpenHuman skill sidecar names and trust marker.

// Host-owned sidecar names and the trust marker are OpenHuman policy.

/// Filename for the OpenHuman skill configuration sidecar.
pub const SKILL_TOML: &str = "skill.toml";

/// Filename for the OpenHuman workflow configuration sidecar.
pub const WORKFLOW_TOML: &str = "workflow.toml";

pub(crate) const TRUST_MARKER: &str = "trust";
