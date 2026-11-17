use serde::Deserialize;

/// Fields shared by the two full agent-chat controller surfaces.
#[derive(Debug, Deserialize)]
pub(crate) struct AgentChatParams {
    pub(crate) message: String,
    pub(crate) model_override: Option<String>,
    pub(crate) temperature: Option<f64>,
    pub(crate) thread_id: Option<String>,
    /// Optional per-turn working directory for filesystem and shell tools.
    pub(crate) cwd: Option<String>,
}
