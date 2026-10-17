use super::*;

/// A flow's LLM capability builds its model under the context it was handed,
/// not the calling task's: the host override of the explicit context wins over
/// an installed ambient model, and two contexts never see each other's model.
#[tokio::test]
async fn create_chat_model_in_reads_the_models_of_the_explicit_context() {
    use crate::agent::host_overrides::HostOverrides;
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    use std::sync::Arc;
    use tinyagents_harness::testkit::ScriptedModel;
    use tinyinference_llm::message::Message;
    use tinyinference_llm::model::ModelRequest;

    let _guard = crate::inference::inference_test_guard_async().await;
    let _override = test_provider_override::install_model(Arc::new(ScriptedModel::replies(vec![
        "echo: ambient",
    ])));
    let config = Config::default();
    let parent = CoreContext::for_test(DomainSet::full(), None);
    let ctx_for = |reply: &'static str| {
        let mut overrides = HostOverrides::default();
        overrides.model = Some(Arc::new(ScriptedModel::replies(vec![reply])));
        let mut overlay = ContextOverlay::new(
            config.clone(),
            DomainSet::full(),
            crate::tools::toolpacks::ToolGroups::default(),
        );
        overlay.host_overrides = Some(Arc::new(overrides));
        DispatchCtx::new(parent.derive_with(overlay))
    };

    for reply in ["echo: one", "echo: two"] {
        let ctx = ctx_for(reply);
        let model = create_chat_model_in(&ctx, "chat", &config, 0.3).expect("explicit model");
        let response = model
            .invoke(&(), ModelRequest::new(vec![Message::user("hi")]))
            .await
            .expect("invoke must succeed");
        assert_eq!(response.text(), reply);
    }
    probe_inference_readiness(&ctx_for("echo: probe"), "chat", &config)
        .await
        .expect("the explicit context's model is ready");
}
