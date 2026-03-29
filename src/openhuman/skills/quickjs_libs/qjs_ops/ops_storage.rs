//! Storage ops: IndexedDB, DB bridge, Store bridge.

use rquickjs::{Ctx, Function, Object};

use super::types::{js_err, SkillContext};
use crate::runtime::quickjs_libs::storage::IdbStorage;

pub fn register<'js>(
    ctx: &Ctx<'js>,
    ops: &Object<'js>,
    storage: IdbStorage,
    skill_context: SkillContext,
) -> rquickjs::Result<()> {
    // ========================================================================
    // IndexedDB (11) - all sync
    // ========================================================================

    {
        let s = storage.clone();
        ops.set(
            "idb_open",
            Function::new(
                ctx.clone(),
                move |name: String, version: u32| -> rquickjs::Result<String> {
                    let result = s.open_database(&name, version).map_err(|e| js_err(e))?;
                    serde_json::to_string(&result).map_err(|e| js_err(e.to_string()))
                },
            ),
        )?;
    }

    {
        let s = storage.clone();
        ops.set(
            "idb_close",
            Function::new(ctx.clone(), move |name: String| {
                s.close_database(&name);
            }),
        )?;
    }

    {
        let s = storage.clone();
        ops.set(
            "idb_delete_database",
            Function::new(ctx.clone(), move |name: String| -> rquickjs::Result<()> {
                s.delete_database(&name).map_err(|e| js_err(e))
            }),
        )?;
    }

    {
        let s = storage.clone();
        ops.set(
            "idb_create_object_store",
            Function::new(
                ctx.clone(),
                move |db_name: String,
                      store_name: String,
                      options: String|
                      -> rquickjs::Result<()> {
                    let opts: serde_json::Value =
                        serde_json::from_str(&options).map_err(|e| js_err(e.to_string()))?;
                    let key_path = opts["keyPath"].as_str();
                    let auto_increment = opts["autoIncrement"].as_bool().unwrap_or(false);
                    s.create_object_store(&db_name, &store_name, key_path, auto_increment)
                        .map_err(|e| js_err(e))
                },
            ),
        )?;
    }

    {
        let s = storage.clone();
        ops.set(
            "idb_delete_object_store",
            Function::new(
                ctx.clone(),
                move |db_name: String, store_name: String| -> rquickjs::Result<()> {
                    s.delete_object_store(&db_name, &store_name)
                        .map_err(|e| js_err(e))
                },
            ),
        )?;
    }

    {
        let s = storage.clone();
        ops.set(
            "idb_get",
            Function::new(
                ctx.clone(),
                move |db_name: String,
                      store_name: String,
                      key: String|
                      -> rquickjs::Result<String> {
                    let key_val: serde_json::Value =
                        serde_json::from_str(&key).map_err(|e| js_err(e.to_string()))?;
                    let result = s
                        .get(&db_name, &store_name, &key_val)
                        .map_err(|e| js_err(e))?;
                    serde_json::to_string(&result).map_err(|e| js_err(e.to_string()))
                },
            ),
        )?;
    }

    {
        let s = storage.clone();
        ops.set(
            "idb_put",
            Function::new(
                ctx.clone(),
                move |db_name: String,
                      store_name: String,
                      key: String,
                      value: String|
                      -> rquickjs::Result<()> {
                    let key_val: serde_json::Value =
                        serde_json::from_str(&key).map_err(|e| js_err(e.to_string()))?;
                    let value_val: serde_json::Value =
                        serde_json::from_str(&value).map_err(|e| js_err(e.to_string()))?;
                    s.put(&db_name, &store_name, &key_val, &value_val)
                        .map_err(|e| js_err(e))
                },
            ),
        )?;
    }

    {
        let s = storage.clone();
        ops.set(
            "idb_delete",
            Function::new(
                ctx.clone(),
                move |db_name: String, store_name: String, key: String| -> rquickjs::Result<()> {
                    let key_val: serde_json::Value =
                        serde_json::from_str(&key).map_err(|e| js_err(e.to_string()))?;
                    s.delete(&db_name, &store_name, &key_val)
                        .map_err(|e| js_err(e))
                },
            ),
        )?;
    }

    {
        let s = storage.clone();
        ops.set(
            "idb_clear",
            Function::new(
                ctx.clone(),
                move |db_name: String, store_name: String| -> rquickjs::Result<()> {
                    s.clear(&db_name, &store_name).map_err(|e| js_err(e))
                },
            ),
        )?;
    }

    {
        let s = storage.clone();
        ops.set(
            "idb_get_all",
            Function::new(
                ctx.clone(),
                move |db_name: String,
                      store_name: String,
                      count: Option<u32>|
                      -> rquickjs::Result<String> {
                    let result = s
                        .get_all(&db_name, &store_name, count)
                        .map_err(|e| js_err(e))?;
                    serde_json::to_string(&result).map_err(|e| js_err(e.to_string()))
                },
            ),
        )?;
    }

    {
        let s = storage.clone();
        ops.set(
            "idb_get_all_keys",
            Function::new(
                ctx.clone(),
                move |db_name: String,
                      store_name: String,
                      count: Option<u32>|
                      -> rquickjs::Result<String> {
                    let result = s
                        .get_all_keys(&db_name, &store_name, count)
                        .map_err(|e| js_err(e))?;
                    serde_json::to_string(&result).map_err(|e| js_err(e.to_string()))
                },
            ),
        )?;
    }

    {
        let s = storage.clone();
        ops.set(
            "idb_count",
            Function::new(
                ctx.clone(),
                move |db_name: String, store_name: String| -> rquickjs::Result<u32> {
                    s.count(&db_name, &store_name).map_err(|e| js_err(e))
                },
            ),
        )?;
    }

    // ========================================================================
    // DB Bridge (5)
    // ========================================================================

    {
        let s = storage.clone();
        let sc = skill_context.clone();
        ops.set(
            "db_exec",
            Function::new(
                ctx.clone(),
                move |sql: String, params_json: Option<String>| -> rquickjs::Result<i64> {
                    let params: Vec<serde_json::Value> = if let Some(p) = params_json {
                        serde_json::from_str(&p).map_err(|e| js_err(e.to_string()))?
                    } else {
                        Vec::new()
                    };
                    let rows = s
                        .skill_db_exec(&sc.skill_id, &sql, &params)
                        .map_err(|e| js_err(e))?;
                    Ok(rows as i64)
                },
            ),
        )?;
    }

    {
        let s = storage.clone();
        let sc = skill_context.clone();
        ops.set(
            "db_get",
            Function::new(
                ctx.clone(),
                move |sql: String, params_json: Option<String>| -> rquickjs::Result<String> {
                    let params: Vec<serde_json::Value> = if let Some(p) = params_json {
                        serde_json::from_str(&p).map_err(|e| js_err(e.to_string()))?
                    } else {
                        Vec::new()
                    };
                    let result = s
                        .skill_db_get(&sc.skill_id, &sql, &params)
                        .map_err(|e| js_err(e))?;
                    serde_json::to_string(&result).map_err(|e| js_err(e.to_string()))
                },
            ),
        )?;
    }

    {
        let s = storage.clone();
        let sc = skill_context.clone();
        ops.set(
            "db_all",
            Function::new(
                ctx.clone(),
                move |sql: String, params_json: Option<String>| -> rquickjs::Result<String> {
                    let params: Vec<serde_json::Value> = if let Some(p) = params_json {
                        serde_json::from_str(&p).map_err(|e| js_err(e.to_string()))?
                    } else {
                        Vec::new()
                    };
                    let result = s
                        .skill_db_all(&sc.skill_id, &sql, &params)
                        .map_err(|e| js_err(e))?;
                    serde_json::to_string(&result).map_err(|e| js_err(e.to_string()))
                },
            ),
        )?;
    }

    {
        let s = storage.clone();
        let sc = skill_context.clone();
        ops.set(
            "db_kv_get",
            Function::new(
                ctx.clone(),
                move |key: String| -> rquickjs::Result<String> {
                    let result = s.skill_kv_get(&sc.skill_id, &key).map_err(|e| js_err(e))?;
                    serde_json::to_string(&result).map_err(|e| js_err(e.to_string()))
                },
            ),
        )?;
    }

    {
        let s = storage.clone();
        let sc = skill_context.clone();
        ops.set(
            "db_kv_set",
            Function::new(
                ctx.clone(),
                move |key: String, value_json: String| -> rquickjs::Result<()> {
                    let value: serde_json::Value =
                        serde_json::from_str(&value_json).map_err(|e| js_err(e.to_string()))?;
                    s.skill_kv_set(&sc.skill_id, &key, &value)
                        .map_err(|e| js_err(e))
                },
            ),
        )?;
    }

    // ========================================================================
    // Store Bridge (4)
    // ========================================================================

    {
        let s = storage.clone();
        let sc = skill_context.clone();
        ops.set(
            "store_get",
            Function::new(
                ctx.clone(),
                move |key: String| -> rquickjs::Result<String> {
                    let result = s
                        .skill_store_get(&sc.skill_id, &key)
                        .map_err(|e| js_err(e))?;
                    serde_json::to_string(&result).map_err(|e| js_err(e.to_string()))
                },
            ),
        )?;
    }

    {
        let s = storage.clone();
        let sc = skill_context.clone();
        ops.set(
            "store_set",
            Function::new(
                ctx.clone(),
                move |key: String, value_json: String| -> rquickjs::Result<()> {
                    let value: serde_json::Value =
                        serde_json::from_str(&value_json).map_err(|e| js_err(e.to_string()))?;
                    s.skill_store_set(&sc.skill_id, &key, &value)
                        .map_err(|e| js_err(e))
                },
            ),
        )?;
    }

    {
        let s = storage.clone();
        let sc = skill_context.clone();
        ops.set(
            "store_delete",
            Function::new(ctx.clone(), move |key: String| -> rquickjs::Result<()> {
                s.skill_store_delete(&sc.skill_id, &key)
                    .map_err(|e| js_err(e))
            }),
        )?;
    }

    {
        let s = storage;
        let sc = skill_context;
        ops.set(
            "store_keys",
            Function::new(ctx.clone(), move || -> rquickjs::Result<String> {
                let keys = s.skill_store_keys(&sc.skill_id).map_err(|e| js_err(e))?;
                serde_json::to_string(&keys).map_err(|e| js_err(e.to_string()))
            }),
        )?;
    }

    Ok(())
}
