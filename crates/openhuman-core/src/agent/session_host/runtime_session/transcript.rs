//! Session transcript identity, locator and metadata construction.

use super::super::types::OpenHumanSessionHost;
use crate::agent::session_store::transcripts_or_files;
use std::sync::Arc;
use tinyagents_session::transcript::TranscriptMeta;

impl OpenHumanSessionHost {
    pub(super) fn runtime_transcript_stem(&self) -> String {
        match &self.session_parent_prefix {
            Some(prefix) => format!("{prefix}__{}", self.session_key),
            None => self.session_key.clone(),
        }
    }

    pub(in crate::agent::session_host) fn session_locator(
        &self,
    ) -> Arc<dyn tinyagents_session::transcript::TranscriptLocator> {
        if let Some(injected) = self.session_history_locator.clone() {
            return injected;
        }
        self.session_history_locator_memo
            .get_or_init(|| {
                let session_agent_id = crate::agent::session_store::current_agent_key_or(
                    &self.dispatch_ctx(),
                    &self.agent_definition_id,
                );
                transcripts_or_files(&session_agent_id, &self.workspace_dir)
            })
            .clone()
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
        }
    }
}
