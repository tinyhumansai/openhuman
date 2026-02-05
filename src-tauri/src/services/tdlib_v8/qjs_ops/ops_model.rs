//! Model ops: local LLM inference via llama-cpp-2.

use rquickjs::{function::Async, Ctx, Function, Object};

use super::types::js_err;

pub fn register(ctx: &Ctx<'_>, ops: &Object<'_>) -> rquickjs::Result<()> {
    ops.set("model_is_available", Function::new(ctx.clone(), || -> bool { false }))?;

    ops.set("model_get_status", Function::new(ctx.clone(), || -> rquickjs::Result<String> {
        let status = crate::services::llama::LLAMA_MANAGER.get_status();
        serde_json::to_string(&status).map_err(|e| js_err(e.to_string()))
    }))?;

    ops.set("model_generate", Function::new(ctx.clone(),
        Async(move |prompt: String, config_json: String| async move {
            let config: crate::services::llama::GenerateConfig =
                serde_json::from_str(&config_json).map_err(|e| js_err(e.to_string()))?;
            crate::services::llama::LLAMA_MANAGER
                .generate(&prompt, config)
                .await
                .map_err(|e| js_err(e))
        }),
    ))?;

    ops.set("model_summarize", Function::new(ctx.clone(),
        Async(move |text: String, max_tokens: u32| async move {
            crate::services::llama::LLAMA_MANAGER
                .summarize(&text, max_tokens)
                .await
                .map_err(|e| js_err(e))
        }),
    ))?;

    Ok(())
}
