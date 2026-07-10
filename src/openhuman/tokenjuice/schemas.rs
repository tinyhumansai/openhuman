//! Read-only RPC controller for inspecting the TokenJuice content router.
//!
//! Diagnostic only — none of these run in the per-tool-output hot path. They
//! let the CLI / debug surfaces see what the router would do (detect a kind,
//! dry-run a compression and show the marker/stats), inspect CCR occupancy, and
//! fetch an offloaded original. Mirrors the controller pattern in
//! `threads/schemas.rs`.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::cache;
use super::compress::route;
use super::detect::detect_content_kind;
use super::tool_integration::current_options;
use super::types::{CompressInput, ContentHint};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("detect"),
        schemas("compress"),
        schemas("cache_stats"),
        schemas("retrieve"),
        schemas("settings_get"),
        schemas("settings_update"),
        schemas("savings_stats"),
        schemas("savings_reset"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("detect"),
            handler: handle_detect,
        },
        RegisteredController {
            schema: schemas("compress"),
            handler: handle_compress,
        },
        RegisteredController {
            schema: schemas("cache_stats"),
            handler: handle_cache_stats,
        },
        RegisteredController {
            schema: schemas("retrieve"),
            handler: handle_retrieve,
        },
        RegisteredController {
            schema: schemas("settings_get"),
            handler: handle_settings_get,
        },
        RegisteredController {
            schema: schemas("settings_update"),
            handler: handle_settings_update,
        },
        RegisteredController {
            schema: schemas("savings_stats"),
            handler: handle_savings_stats,
        },
        RegisteredController {
            schema: schemas("savings_reset"),
            handler: handle_savings_reset,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "detect" => ControllerSchema {
            namespace: "tokenjuice",
            function: "detect",
            description:
                "Detect the content kind of a blob (json/code/log/search/diff/html/plain_text).",
            inputs: vec![
                FieldSchema {
                    name: "content",
                    ty: TypeSchema::String,
                    comment: "The content to classify.",
                    required: true,
                },
                FieldSchema {
                    name: "tool_name",
                    ty: TypeSchema::String,
                    comment: "Optional producing tool name (prior hint).",
                    required: false,
                },
                FieldSchema {
                    name: "extension",
                    ty: TypeSchema::String,
                    comment: "Optional file extension hint (no dot).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "kind",
                ty: TypeSchema::String,
                comment: "Detected content kind.",
                required: true,
            }],
        },
        "compress" => ControllerSchema {
            namespace: "tokenjuice",
            function: "compress",
            description: "Dry-run the content router over a blob and report stats + marker.",
            inputs: vec![
                FieldSchema {
                    name: "content",
                    ty: TypeSchema::String,
                    comment: "The content to compress.",
                    required: true,
                },
                FieldSchema {
                    name: "tool_name",
                    ty: TypeSchema::String,
                    comment: "Optional producing tool name (prior hint).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Applied flag, kind, compressor, byte counts, CCR token, and text.",
                required: true,
            }],
        },
        "cache_stats" => ControllerSchema {
            namespace: "tokenjuice",
            function: "cache_stats",
            description: "Report CCR cache occupancy (entry count and total bytes).",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "{ entries, bytes }.",
                required: true,
            }],
        },
        "retrieve" => ControllerSchema {
            namespace: "tokenjuice",
            function: "retrieve",
            description: "Fetch a previously-offloaded original from the CCR cache by token.",
            inputs: vec![FieldSchema {
                name: "token",
                ty: TypeSchema::String,
                comment: "The CCR token (hash) from a ⟦tj:…⟧ marker.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "{ found, content }.",
                required: true,
            }],
        },
        "settings_get" => ControllerSchema {
            namespace: "tokenjuice",
            function: "settings_get",
            description: "Get the current [tokenjuice] configuration block.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "settings",
                ty: TypeSchema::Json,
                comment: "The tokenjuice config (router/CCR/compressor toggles + ML fields).",
                required: true,
            }],
        },
        "settings_update" => ControllerSchema {
            namespace: "tokenjuice",
            function: "settings_update",
            description: "Patch the [tokenjuice] config (any subset of fields), persist, and apply live.",
            inputs: vec![FieldSchema {
                name: "patch",
                ty: TypeSchema::Json,
                comment: "Partial tokenjuice settings; only present fields are changed.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "settings",
                ty: TypeSchema::Json,
                comment: "The full settings after applying the patch.",
                required: true,
            }],
        },
        "savings_stats" => ControllerSchema {
            namespace: "tokenjuice",
            function: "savings_stats",
            description: "Token + cost savings the content router has accrued (total + by model + by compressor).",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "{ attributionModel, total, byModel, byCompressor, cache }.",
                required: true,
            }],
        },
        "savings_reset" => ControllerSchema {
            namespace: "tokenjuice",
            function: "savings_reset",
            description: "Clear all recorded savings statistics.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "ok",
                ty: TypeSchema::Bool,
                comment: "True once reset.",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "tokenjuice",
            function: "unknown",
            description: "Unknown tokenjuice controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

// ── Handlers ─────────────────────────────────────────────────────────

fn str_param(params: &Map<String, Value>, key: &str) -> Option<String> {
    params.get(key).and_then(Value::as_str).map(str::to_string)
}

fn handle_detect(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let content = str_param(&params, "content").ok_or("missing 'content'")?;
        let hint = ContentHint {
            source_tool: str_param(&params, "tool_name"),
            extension: str_param(&params, "extension"),
            ..Default::default()
        };
        let kind = detect_content_kind(&content, &hint);
        Ok(serde_json::json!({ "kind": kind.as_str() }))
    })
}

fn handle_compress(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let content = str_param(&params, "content").ok_or("missing 'content'")?;
        let hint = ContentHint {
            source_tool: str_param(&params, "tool_name"),
            ..Default::default()
        };
        let opts = current_options();
        let input = CompressInput {
            content: &content,
            kind: super::types::ContentKind::PlainText,
            hint: &hint,
            exit_code: None,
            command: None,
            argv: None,
            original_bytes: content.len(),
        };
        let res = route(input, &opts).await;
        Ok(serde_json::json!({
            "applied": res.applied,
            "kind": res.content_kind.as_str(),
            "compressor": res.compressor.as_str(),
            "lossy": res.lossy,
            "originalBytes": res.original_bytes,
            "compactedBytes": res.compacted_bytes,
            "ccrToken": res.ccr_token,
            "text": res.text,
        }))
    })
}

fn handle_cache_stats(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let (entries, bytes) = cache::stats();
        Ok(serde_json::json!({ "entries": entries, "bytes": bytes }))
    })
}

fn handle_retrieve(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let token = str_param(&params, "token").ok_or("missing 'token'")?;
        match cache::retrieve(&token) {
            Some(content) => Ok(serde_json::json!({ "found": true, "content": content })),
            None => Ok(serde_json::json!({ "found": false, "content": Value::Null })),
        }
    })
}

fn handle_settings_get(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = crate::openhuman::config::Config::load_or_init()
            .await
            .map_err(|e| format!("load config: {e}"))?;
        let settings = serde_json::to_value(&config.tokenjuice)
            .map_err(|e| format!("serialize tokenjuice settings: {e}"))?;
        Ok(serde_json::json!({ "settings": settings }))
    })
}

fn handle_settings_update(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        // Accept either {"patch": {...}} or a bare object of fields.
        let patch = params
            .get("patch")
            .cloned()
            .unwrap_or_else(|| Value::Object(params.clone()));
        let patch: super::config_patch::TokenjuiceSettingsPatch =
            serde_json::from_value(patch).map_err(|e| format!("invalid patch: {e}"))?;

        let mut config = crate::openhuman::config::Config::load_or_init()
            .await
            .map_err(|e| format!("load config: {e}"))?;
        patch.apply(&mut config.tokenjuice);
        config
            .save()
            .await
            .map_err(|e| format!("save config: {e}"))?;

        // Re-install so router flags / CCR limits / threshold take effect live.
        crate::openhuman::tokenjuice::install_from_config(&config);

        let settings = serde_json::to_value(&config.tokenjuice)
            .map_err(|e| format!("serialize tokenjuice settings: {e}"))?;
        Ok(serde_json::json!({ "settings": settings }))
    })
}

fn handle_savings_stats(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let agg = super::savings::stats();
        let (entries, bytes) = cache::stats();
        Ok(serde_json::json!({
            "attributionModel": super::savings::attribution_model(),
            "total": agg.total,
            "byModel": agg.by_model,
            "byCompressor": agg.by_compressor,
            "cache": { "entries": entries, "bytes": bytes },
        }))
    })
}

fn handle_savings_reset(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        super::savings::reset();
        Ok(serde_json::json!({ "ok": true }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn detect_handler_classifies_json() {
        let mut p = Map::new();
        p.insert(
            "content".into(),
            Value::String(r#"[{"a":1,"b":2},{"a":3,"b":4}]"#.into()),
        );
        let out = handle_detect(p).await.unwrap();
        assert_eq!(out["kind"], "json");
    }

    #[tokio::test]
    async fn cache_stats_handler_returns_counts() {
        cache::offload("tokenjuice controller stats unique payload here");
        let out = handle_cache_stats(Map::new()).await.unwrap();
        assert!(out["entries"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn all_schemas_have_namespace() {
        for s in all_controller_schemas() {
            assert_eq!(s.namespace, "tokenjuice");
        }
    }
}
