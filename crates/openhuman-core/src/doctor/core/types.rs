//! Stable wire types for the currently unavailable model-probe method.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelProbeOutcome {
    Ok,
    Skipped,
    AuthOrAccess,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProbeEntry {
    pub provider: String,
    pub outcome: ModelProbeOutcome,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProbeSummary {
    pub ok: usize,
    pub skipped: usize,
    pub auth_or_access: usize,
    pub errors: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProbeReport {
    pub entries: Vec<ModelProbeEntry>,
    pub summary: ModelProbeSummary,
}
