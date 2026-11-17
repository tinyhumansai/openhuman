//! OpenHuman copy overrides for TinyAgents' provider-neutral stop summaries.

use tinyagents_harness::turn_stop::{CopyProvider, CopyRequest, TurnStopCopyKey};

pub(crate) struct OpenHumanCopyProvider;

impl CopyProvider for OpenHumanCopyProvider {
    fn copy_override(&self, request: &CopyRequest<'_>) -> Option<String> {
        match request {
            CopyRequest::ProviderConfig { tool, last_error } => Some(format!(
                "Stopping: the `{tool}` step failed because the configured model/provider rejected the request (e.g. an unknown model, a non-chat/embedding model, a missing credential, or a region block) — retrying will not help. Fix the model or API key in Connections → API keys → LLM. Details:\n{last_error}"
            )),
            CopyRequest::TurnStop {
                key: TurnStopCopyKey::ProviderConfig,
                ..
            } => Some(
                "the configured model or provider rejected the request; check the model and API key in Connections".into(),
            ),
            CopyRequest::TurnStop {
                key: TurnStopCopyKey::MissingConnection,
                ..
            } => Some(
                "a service this needs isn't connected; connect it in Connections, then ask me to try again".into(),
            ),
            _ => None,
        }
    }
}
