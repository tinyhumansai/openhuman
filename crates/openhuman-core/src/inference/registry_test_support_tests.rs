//! Test support: a fake managed chat model standing in for the hosted provider
//! the core has no copy of. [`chat_model`](super::registry) falls back to it in
//! unit tests, so the many tests that build a default-config agent keep
//! working; a test of the bare core opts out with [`without_managed_provider`].

use std::sync::Arc;

use async_trait::async_trait;
use tinyinference_llm::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};

use super::registry::{ChatModelEntry, ChatModelRequest};
use crate::inference::provider::factory::PROVIDER_OPENHUMAN;

/// A chat model whose profile mirrors the managed provider's (`managed`, with
/// tool capabilities following `native_tool_calling`).
pub(crate) struct FakeManagedModel {
    profile: ModelProfile,
    /// The thread the request that built this model carried.
    pub(crate) thread_id: Option<String>,
}

#[async_trait]
impl ChatModel<()> for FakeManagedModel {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(&self.profile)
    }

    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        // Like the real provider with no stored session: construction works,
        // a call fails.
        Err(tinyinference_llm::Error::Model(
            "No backend session: store a JWT via auth (app-session)".to_string(),
        ))
    }
}

fn build(request: &ChatModelRequest<'_>) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    let native = request.native_tool_calling;
    let model = FakeManagedModel {
        profile: ModelProfile {
            provider: Some("managed".to_string()),
            tool_calling: native,
            parallel_tool_calls: native,
            streaming_tool_chunks: native,
            streaming: true,
            ..ModelProfile::default()
        },
        thread_id: request.thread_id.map(str::to_string),
    };
    let resolved = request
        .model_override
        .unwrap_or(crate::config::MODEL_MANAGED_DEFAULT)
        .to_string();
    Ok((Arc::new(model), resolved))
}

thread_local! {
    static BARE_CORE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The managed provider every unit test sees unless it opts out: the fake,
/// answering for [`PROVIDER_OPENHUMAN`] only.
pub(crate) fn default_managed_entry(provider_id: &str) -> Option<ChatModelEntry> {
    if provider_id != PROVIDER_OPENHUMAN || BARE_CORE.with(std::cell::Cell::get) {
        return None;
    }
    Some(ChatModelEntry {
        build: Arc::new(build),
        probe: None,
    })
}

/// Guard that makes the current thread see a core with no managed chat-model
/// provider (the default registry state of a host that installed none).
pub(crate) struct BareCore;

/// Runs the rest of the test on a core with no managed chat-model provider.
pub(crate) fn without_managed_provider() -> BareCore {
    BARE_CORE.with(|flag| flag.set(true));
    BareCore
}

impl Drop for BareCore {
    fn drop(&mut self) {
        BARE_CORE.with(|flag| flag.set(false));
    }
}
