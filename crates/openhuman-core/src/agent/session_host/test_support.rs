use super::*;
use tinyinference_llm::providers::overrides::{
    ChatModelEntry, ChatModelRequest, ProviderModelMetadata,
};

use crate::inference::provider::factory::PROVIDER_OPENHUMAN;
use crate::inference::registry::ProviderContext;
use std::sync::Arc;
use tinyinference_llm::model::{ChatModel, ModelRequest, ModelResponse};

struct SessionFixtureModel;

#[async_trait::async_trait]
impl ChatModel<()> for SessionFixtureModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        Ok(ModelResponse::assistant("session fixture"))
    }
}

pub(crate) fn with_fixture_model_name(builder: SessionHostBuilder) -> SessionHostBuilder {
    builder.model_name("test/session-fixture".to_string())
}

/// Installs an explicit generic model fixture for tests that construct a
/// session through the managed-provider factory. HookSlot test registrations
/// are thread-local, so each test opts in on the thread that builds its host.
fn model_fixture_entry() -> ChatModelEntry<ProviderContext> {
    scripted_model_entry(Arc::new(SessionFixtureModel))
}

fn scripted_model_entry(scripted: Arc<dyn ChatModel<()>>) -> ChatModelEntry<ProviderContext> {
    ChatModelEntry {
        build: Arc::new(move |request: &ChatModelRequest<'_, ProviderContext>| {
            let model = request
                .model_override
                .or(request.default_model)
                .or(request.context.config.default_model.as_deref())
                .unwrap_or("test/session-fixture");
            Ok((Arc::clone(&scripted), model.to_string()))
        }),
        probe: None,
        models: Some(ProviderModelMetadata {
            default_model: Arc::new(|| Some("test/session-fixture".into())),
            role_for_model: Arc::new(|model| {
                if model == "fixture-reasoning" {
                    Some("reasoning".to_string())
                } else {
                    model
                        .strip_prefix("hint:")
                        .filter(|role| crate::config::WORKLOAD_ROLES.contains(role))
                        .map(str::to_string)
                }
            }),
            is_alias: Arc::new(|model| {
                model == "fixture-reasoning"
                    || model
                        .strip_prefix("hint:")
                        .is_some_and(|role| crate::config::WORKLOAD_ROLES.contains(&role))
            }),
            is_raw_model: Arc::new(|model| {
                !model.starts_with("hint:") && model != "fixture-reasoning"
            }),
            supports_vision: Arc::new(|_| false),
            resolve_model: Arc::new(|role, config, model| match model {
                Some("fixture-reasoning") => Some("test/selected-reasoning".to_string()),
                Some(model) if !model.starts_with("hint:") => Some(model.to_string()),
                Some(model) => model
                    .strip_prefix("hint:")
                    .filter(|hint_role| crate::config::WORKLOAD_ROLES.contains(hint_role))
                    .map(|hint_role| {
                        if hint_role == "chat" {
                            config
                                .config
                                .default_model
                                .as_deref()
                                .filter(|configured| !configured.starts_with("hint:"))
                                .map(str::to_string)
                                .unwrap_or_else(|| "test/session-fixture".to_string())
                        } else {
                            "test/session-fixture".to_string()
                        }
                    }),
                None if role == "agentic" => Some("test/session-fixture".into()),
                None => match config.config.default_model.as_deref() {
                    Some("fixture-reasoning") => Some("test/selected-reasoning".into()),
                    Some(model) => Some(model.to_string()),
                    None => Some("test/session-fixture".into()),
                },
            }),
            context_window: Arc::new(|_| None),
            default_pricing: Arc::new(|| None),
            without_reasoning: Arc::new(|request| request),
        }),
    }
}

pub(crate) fn install_model_fixture() {
    crate::inference::registry::register_chat_model(PROVIDER_OPENHUMAN, model_fixture_entry());
}

/// Configure a managed-provider session fixture with its required test credential.
pub(crate) async fn install_managed_model_fixture(config: &crate::config::Config) {
    install_model_fixture();
    crate::credentials::api_key::store_api_key(config, "test-session-fixture-key")
        .await
        .expect("store the managed inference fixture credential");
}

pub(crate) fn install_scripted_model(model: Arc<dyn ChatModel<()>>) {
    crate::inference::registry::register_chat_model(
        PROVIDER_OPENHUMAN,
        scripted_model_entry(model),
    );
}

pub(crate) fn install_model_fixture_in(extensions: &crate::core::slots::Extensions) {
    crate::inference::registry::register_chat_model_in(
        extensions,
        PROVIDER_OPENHUMAN,
        model_fixture_entry(),
    );
}
