use super::*;
use std::sync::Arc;

#[tokio::test]
async fn builder_requires_a_model_when_no_host_policy_is_installed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config_without_inference(&tmp);
    assert!(config.default_model.is_none());
    let model: Arc<dyn tinyinference_llm::model::ChatModel<()>> =
        Arc::new(tinyagents_harness::testkit::ScriptedModel::new(Vec::new()));
    let error = crate::agent::SessionHostBuilder::new()
        .chat_model(model)
        .tools(Vec::new())
        .tool_dispatcher(Box::new(tinytools_agent::dialect::XmlDialect))
        .workspace_dir(config.workspace_dir)
        .build()
        .err()
        .expect("a bare core builder needs an explicit model");

    assert!(error
        .to_string()
        .contains("model_name or configured provider default is required"));
}
