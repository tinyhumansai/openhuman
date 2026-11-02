use super::*;

#[tokio::test]
async fn default_temperature_wrapper_preserves_physical_attempt_tracking() {
    use async_trait::async_trait;
    use std::sync::Arc;
    use tinyinference_llm::model::{ModelRequest, ModelResponse};

    struct TrackedModel;

    #[async_trait]
    impl ChatModel<()> for TrackedModel {
        fn tracks_physical_attempts(&self) -> bool {
            true
        }

        fn has_pre_invocation_admission_gate(&self) -> bool {
            true
        }

        async fn invoke(
            &self,
            _state: &(),
            _request: ModelRequest,
        ) -> tinyinference_llm::Result<ModelResponse> {
            Ok(ModelResponse::assistant("ok"))
        }
    }

    let _guard = crate::inference::inference_test_guard();
    let _override = test_provider_override::install_model(Arc::new(TrackedModel));
    let wrapped = create_chat_model_in(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        &Config::default(),
        0.3,
    )
    .await
    .expect("test override model");

    assert!(wrapped.tracks_physical_attempts());
    assert!(wrapped.has_pre_invocation_admission_gate());
}
