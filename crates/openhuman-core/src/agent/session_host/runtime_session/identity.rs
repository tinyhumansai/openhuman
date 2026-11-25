//! OpenHuman's tenant-aware admission for TinyAgents session stores.

use std::path::PathBuf;

use tinyagents_runtime::{
    ResolvedSessionStores, RuntimeError, SessionIdentityRequest, SessionIdentityResolver,
};
use tinyagents_session::transcript::TranscriptMeta;

use super::super::types::OpenHumanSessionHost;
use crate::agent::tinyagents::host::OpenHumanRunContext;

impl OpenHumanSessionHost {
    pub(super) fn injected_transcript_stem(&self) -> String {
        match &self.session_parent_prefix {
            Some(prefix) => format!("{prefix}__{}", self.session_key),
            None => self.session_key.clone(),
        }
    }

    pub(super) fn runtime_session_key(&self) -> String {
        self.thread_id
            .clone()
            .unwrap_or_else(|| match &self.session_parent_prefix {
                Some(prefix) => format!("{prefix}__{}", self.session_key),
                None => self.session_key.clone(),
            })
    }

    pub(super) fn session_identity_resolver(&self) -> OpenHumanSessionIdentityResolver {
        OpenHumanSessionIdentityResolver {
            fallback_agent_id: self.agent_definition_id.clone(),
            fallback_workspace: self.workspace_dir.clone(),
        }
    }

    pub(super) fn subagent_transcript_locator(
        &self,
    ) -> std::sync::Arc<dyn tinyagents_session::transcript::TranscriptLocator> {
        if let Some(locator) = &self.session_history_locator {
            return std::sync::Arc::clone(locator);
        }
        let dispatch = self.dispatch_ctx();
        let workspace = dispatch
            .resolved()
            .and_then(|core| core.workspace_dir().ok())
            .unwrap_or_else(|| self.workspace_dir.clone());
        let agent_id =
            crate::storage::sessions::current_agent_key_or(&dispatch, &self.agent_definition_id);
        crate::storage::sessions::provider_for_workspace_in(&workspace, &dispatch)
            .for_agent(&agent_id)
            .transcripts
    }

    pub(super) fn runtime_transcript_meta(&self) -> TranscriptMeta {
        let now = chrono::Utc::now().to_rfc3339();
        TranscriptMeta {
            agent_name: self.agent_definition_name.clone(),
            agent_id: Some(self.agent_definition_id.clone()),
            agent_type: Some(if self.session_parent_prefix.is_some() {
                "subagent".into()
            } else {
                "root".into()
            }),
            dispatcher: if self.tool_dispatcher.should_send_tool_specs() {
                "native".into()
            } else {
                "xml".into()
            },
            provider: None,
            model: Some(self.model_name.clone()),
            created: now.clone(),
            updated: now,
            turn_count: 0,
            prefix_message_count: None,
            input_tokens: 0,
            output_tokens: 0,
            cached_input_tokens: 0,
            charged_amount_usd: 0.0,
            thread_id: self.thread_id.clone(),
            task_id: None,
            session_id: self.session.as_ref().map(|session| session.session_id()),
            parent_session_id: self
                .session
                .as_ref()
                .and_then(|session| session.parent_session_id()),
            prefix_reset: false,
        }
    }
}

pub(super) struct OpenHumanSessionIdentityResolver {
    fallback_agent_id: String,
    fallback_workspace: PathBuf,
}

impl SessionIdentityResolver<OpenHumanRunContext> for OpenHumanSessionIdentityResolver {
    fn resolve(
        &self,
        _request: &SessionIdentityRequest,
        context: &OpenHumanRunContext,
    ) -> Result<ResolvedSessionStores, RuntimeError> {
        let dispatch = context.dispatch_ctx();
        let tenant = dispatch.tenant().map_err(|error| {
            RuntimeError::InvalidSessionState(format!("session tenant is unavailable: {error}"))
        })?;
        let agent_id = crate::storage::sessions::agent_key_for(&tenant, &self.fallback_agent_id);
        let workspace = dispatch
            .resolved()
            .and_then(|core| core.workspace_dir().ok())
            .unwrap_or_else(|| self.fallback_workspace.clone());
        let provider = crate::storage::sessions::provider_for_workspace_in(&workspace, &dispatch);
        let stores = provider.lease_for_agent(&agent_id).map_err(|error| {
            RuntimeError::Persistence(format!("could not admit session stores: {error}"))
        })?;
        Ok(ResolvedSessionStores { agent_id, stores })
    }
}
