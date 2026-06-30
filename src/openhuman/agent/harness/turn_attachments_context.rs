//! Task-local carrier for the **current turn's image-attachment placeholders**
//! (`[Image: … #att:<id>]`), so a delegation to a vision sub-agent can forward
//! the user's attached image into the sub-agent's prompt.
//!
//! The orchestrator runs on a non-vision tier (`chat-v1`): its turn keeps the
//! image as a text placeholder and never rehydrates it (see the image gate in
//! [`crate::openhuman::agent::harness::engine`]). When it delegates to the
//! vision sub-agent via `analyze_image`, the sub-agent only receives the
//! orchestrator's text task — not the conversation history. This task-local
//! surfaces the current user message's placeholders to
//! [`crate::openhuman::agent_orchestration::tools::dispatch`], which prepends
//! them to the sub-agent prompt so the (vision-capable) sub-agent's turn
//! rehydrates the image from the on-disk sidecar.
//!
//! Mirrors [`super::model_vision_context`]. Scoped around the orchestrator's
//! `run_turn_engine` call; [`current_turn_image_placeholders`] returns an empty
//! vec when no scope is active (CLI / direct invocation / tests) — strictly
//! additive.

tokio::task_local! {
    /// Image-attachment placeholder tokens from the current turn's user message.
    pub static CURRENT_TURN_IMAGE_PLACEHOLDERS: Vec<String>;
}

/// Placeholders for the current turn, or an empty vec when no scope is active.
pub fn current_turn_image_placeholders() -> Vec<String> {
    CURRENT_TURN_IMAGE_PLACEHOLDERS
        .try_with(|v| v.clone())
        .unwrap_or_default()
}

/// Run `future` with `placeholders` installed as the current turn's image
/// placeholders. Intended call site is around the orchestrator's
/// `run_turn_engine` invocation.
pub async fn with_current_turn_image_placeholders<F, R>(placeholders: Vec<String>, future: F) -> R
where
    F: std::future::Future<Output = R>,
{
    CURRENT_TURN_IMAGE_PLACEHOLDERS
        .scope(placeholders, future)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_outside_scope() {
        assert!(current_turn_image_placeholders().is_empty());
    }

    #[tokio::test]
    async fn installs_and_reads_back() {
        let observed = with_current_turn_image_placeholders(
            vec!["[Image: image #att:abc123]".to_string()],
            async { current_turn_image_placeholders() },
        )
        .await;
        assert_eq!(observed, vec!["[Image: image #att:abc123]".to_string()]);
    }

    #[tokio::test]
    async fn does_not_leak_across_scopes() {
        with_current_turn_image_placeholders(vec!["x".to_string()], async {
            assert_eq!(current_turn_image_placeholders().len(), 1);
        })
        .await;
        assert!(current_turn_image_placeholders().is_empty());
    }
}
