//! TDLib ops: Telegram database library integration (gated on skill_id == "telegram").

use rquickjs::{function::Async, Ctx, Function, Object};
use std::path::PathBuf;

use super::types::{check_telegram_skill, js_err, SkillContext};

pub fn register(ctx: &Ctx<'_>, ops: &Object<'_>, skill_context: SkillContext) -> rquickjs::Result<()> {
    {
        let sc = skill_context.clone();
        ops.set("tdlib_is_available", Function::new(ctx.clone(),
            move || -> bool { sc.skill_id == "telegram" },
        ))?;
    }

    {
        let sc = skill_context.clone();
        ops.set("tdlib_create_client", Function::new(ctx.clone(),
            move |data_dir: String| -> rquickjs::Result<i32> {
                check_telegram_skill(&sc.skill_id).map_err(|e| js_err(e))?;
                crate::services::tdlib::TDLIB_MANAGER
                    .create_client(PathBuf::from(data_dir))
                    .map_err(|e| js_err(e))
            },
        ))?;
    }

    {
        let sc = skill_context.clone();
        ops.set("tdlib_send", Function::new(ctx.clone(),
            Async(move |request_json: String| {
                let skill_id = sc.skill_id.clone();
                async move {
                    check_telegram_skill(&skill_id).map_err(|e| js_err(e))?;
                    let request: serde_json::Value =
                        serde_json::from_str(&request_json).map_err(|e| js_err(e.to_string()))?;
                    let result = crate::services::tdlib::TDLIB_MANAGER
                        .send(request)
                        .await
                        .map_err(|e| js_err(e))?;
                    serde_json::to_string(&result).map_err(|e| js_err(e.to_string()))
                }
            }),
        ))?;
    }

    {
        let sc = skill_context.clone();
        ops.set("tdlib_receive", Function::new(ctx.clone(),
            Async(move |timeout_ms: u32| {
                let skill_id = sc.skill_id.clone();
                async move {
                    check_telegram_skill(&skill_id).map_err(|e| js_err(e))?;
                    let result = crate::services::tdlib::TDLIB_MANAGER.receive(timeout_ms).await;
                    if let Some(val) = result {
                        let json = serde_json::to_string(&val).map_err(|e| js_err(e.to_string()))?;
                        Ok::<Option<String>, rquickjs::Error>(Some(json))
                    } else {
                        Ok(None)
                    }
                }
            }),
        ))?;
    }

    {
        let sc = skill_context;
        ops.set("tdlib_destroy", Function::new(ctx.clone(),
            Async(move || {
                let skill_id = sc.skill_id.clone();
                async move {
                    check_telegram_skill(&skill_id).map_err(|e| js_err(e))?;
                    crate::services::tdlib::TDLIB_MANAGER.destroy().await.map_err(|e| js_err(e))
                }
            }),
        ))?;
    }

    Ok(())
}
