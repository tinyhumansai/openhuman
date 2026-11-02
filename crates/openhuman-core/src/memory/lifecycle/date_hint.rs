//! OpenHuman's model callback for TinyMemory's runtime-neutral date hint.

use std::sync::Arc;

use futures::future::BoxFuture;
use tinymemory_tools::lifecycle::date_hint::DateHintModel;

use crate::config::Config;
use crate::core::runtime::DispatchCtx;

pub(super) fn model(ctx: DispatchCtx, config: Arc<Config>) -> Arc<dyn DateHintModel> {
    Arc::new(HostDateHintModel { ctx, config })
}

struct HostDateHintModel {
    ctx: DispatchCtx,
    config: Arc<Config>,
}

impl DateHintModel for HostDateHintModel {
    fn complete(
        &self,
        instruction: String,
        user_text: String,
        zone: String,
    ) -> BoxFuture<'static, Option<String>> {
        let ctx = self.ctx.clone();
        let config = self.config.clone();
        Box::pin(async move {
            let (model, model_id) =
                match crate::inference::provider::create_chat_model_with_model_id_in(
                    &ctx, "chat", &config, 0.0,
                )
                .await
                {
                    Ok(model) => model,
                    Err(error) => {
                        tracing::debug!(%error, "[memory:date_hint] no chat model");
                        return None;
                    }
                };
            let system = format!(
                "{}\n\n{instruction}",
                crate::prompts::current_datetime_line(Some(&zone))
            );
            let mut request = tinyinference_llm::model::ModelRequest::new(vec![
                tinyinference_llm::message::Message::system(system),
                tinyinference_llm::message::Message::user(user_text),
            ]);
            request.max_tokens = Some(400);
            request.temperature = Some(0.0);
            let started = std::time::Instant::now();
            match model.invoke(&(), request).await {
                Ok(response) => {
                    let elapsed_ms = started.elapsed().as_millis() as u64;
                    tracing::debug!(
                        model = %model_id,
                        elapsed_ms,
                        "[memory:date_hint] model answered"
                    );
                    Some(response.text())
                }
                Err(error) => {
                    let elapsed_ms = started.elapsed().as_millis() as u64;
                    tracing::debug!(
                        model = %model_id,
                        elapsed_ms,
                        %error,
                        "[memory:date_hint] call failed"
                    );
                    None
                }
            }
        })
    }
}
