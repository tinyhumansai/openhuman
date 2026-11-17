//! Static per-turn helpers: iteration counting, history diffing, event-error
//! sanitisation, and tool-call id fallback / persistence shaping.

use super::super::types::OpenHumanSessionHost;
use crate::util::truncate_with_ellipsis;
use tinyagents_harness::agent_error::AgentError;

impl OpenHumanSessionHost {
    const EVENT_ERROR_MAX_CHARS: usize = 256;

    // The turn count comes from the durable runtime receipt. This module keeps
    // only OpenHuman's safe event projection for untyped failures.

    pub(in crate::agent::session_host) fn sanitize_event_error_message(
        err: &anyhow::Error,
    ) -> String {
        if let Some(error) = err.downcast_ref::<AgentError>() {
            let kind = error.kind();
            if kind != "other" {
                return kind.to_string();
            }
        }

        let scrubbed = tinyinference_core::sanitize::sanitize_api_error(&err.to_string())
            .replace(['\n', '\r', '\t'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        truncate_with_ellipsis(&scrubbed, Self::EVENT_ERROR_MAX_CHARS)
    }
}
