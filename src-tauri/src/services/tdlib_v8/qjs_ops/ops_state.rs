//! State and data ops: published state get/set, filesystem data read/write.

use parking_lot::RwLock;
use rquickjs::{Ctx, Function, Object};
use std::sync::Arc;

use super::types::{js_err, SkillContext, SkillState};

pub fn register(
    ctx: &Ctx<'_>,
    ops: &Object<'_>,
    skill_state: Arc<RwLock<SkillState>>,
    skill_context: SkillContext,
) -> rquickjs::Result<()> {
    // ========================================================================
    // State Bridge (3)
    // ========================================================================

    {
        let ss = skill_state.clone();
        ops.set("state_get", Function::new(ctx.clone(),
            move |key: String| -> rquickjs::Result<String> {
                let state = ss.read();
                let value = state.data.get(&key).cloned().unwrap_or(serde_json::Value::Null);
                serde_json::to_string(&value).map_err(|e| js_err(e.to_string()))
            },
        ))?;
    }

    {
        let ss = skill_state.clone();
        ops.set("state_set", Function::new(ctx.clone(),
            move |key: String, value_json: String| -> rquickjs::Result<()> {
                let value: serde_json::Value =
                    serde_json::from_str(&value_json).map_err(|e| js_err(e.to_string()))?;
                let mut state = ss.write();
                state.data.insert(key, value);
                Ok(())
            },
        ))?;
    }

    {
        let ss = skill_state;
        ops.set("state_set_partial", Function::new(ctx.clone(),
            move |partial_json: String| -> rquickjs::Result<()> {
                let partial: serde_json::Map<String, serde_json::Value> =
                    serde_json::from_str(&partial_json).map_err(|e| js_err(e.to_string()))?;
                let mut state = ss.write();
                for (k, v) in partial {
                    state.data.insert(k, v);
                }
                Ok(())
            },
        ))?;
    }

    // ========================================================================
    // Data Bridge (2)
    // ========================================================================

    {
        let sc = skill_context.clone();
        ops.set("data_read", Function::new(ctx.clone(),
            move |filename: String| -> rquickjs::Result<String> {
                let path = sc.data_dir.join(&filename);
                std::fs::read_to_string(&path).map_err(|e| js_err(e.to_string()))
            },
        ))?;
    }

    {
        let sc = skill_context;
        ops.set("data_write", Function::new(ctx.clone(),
            move |filename: String, content: String| -> rquickjs::Result<()> {
                let path = sc.data_dir.join(&filename);
                std::fs::write(&path, content).map_err(|e| js_err(e.to_string()))
            },
        ))?;
    }

    Ok(())
}
