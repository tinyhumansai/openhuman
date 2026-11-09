use super::*;
use std::sync::{Arc, Mutex};
use tinyinference_llm::message::Message;
use tinyinference_llm::model::ModelRequest;

struct RequestProbe {
    request: Mutex<Option<ModelRequest>>,
}

#[async_trait::async_trait]
impl ChatModel<()> for RequestProbe {
    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        *self.request.lock().expect("probe lock") = Some(request);
        Ok(ModelResponse::assistant("reply"))
    }
}

#[tokio::test]
async fn simple_chat_uses_resolved_model_and_temperature_defaults() {
    let probe = Arc::new(RequestProbe {
        request: Mutex::new(None),
    });
    let model: Arc<dyn ChatModel<()>> = probe.clone();

    let response =
        simple_chat_with_model(model, "resolved/provider-model".to_string(), 0.37, "hello")
            .await
            .expect("simple chat response");

    assert_eq!(response.text(), "reply");
    let request = probe
        .request
        .lock()
        .expect("probe lock")
        .take()
        .expect("model invocation");
    assert_eq!(request.model.as_deref(), Some("resolved/provider-model"));
    assert_eq!(request.temperature, Some(0.37));
    assert_eq!(request.messages, vec![Message::user("hello")]);
}
