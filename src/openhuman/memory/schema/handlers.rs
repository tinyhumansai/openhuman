//! Handler functions for every `memory_tree` JSON-RPC method.
//!
//! Each `handle_*` function is a thin bridge from raw JSON params to the
//! typed RPC calls in [`crate::openhuman::memory_tree::tree::rpc`] (write
//! side) or [`crate::openhuman::memory::read_rpc`] (UI read side).

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::ControllerFuture;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory::read_rpc;
use crate::openhuman::memory_tree::tree::rpc;
use crate::rpc::RpcOutcome;

// ── Write-side handlers (rpc::*) ─────────────────────────────────────────

pub(super) fn handle_ingest(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<rpc::IngestRequest>(Value::Object(params))?;
        to_json(rpc::ingest_rpc(&config, req).await?)
    })
}

pub(super) fn handle_get_chunk(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<rpc::GetChunkRequest>(Value::Object(params))?;
        to_json(rpc::get_chunk_rpc(&config, req).await?)
    })
}

pub(super) fn handle_memory_backfill_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(rpc::backfill_status_rpc(&config).await?)
    })
}

// ── Read-side handlers (read_rpc::*) ─────────────────────────────────────

pub(super) fn handle_list_chunks(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let filter = parse_value::<read_rpc::ChunkFilter>(Value::Object(params))?;
        to_json(read_rpc::list_chunks_rpc(&config, filter).await?)
    })
}

pub(super) fn handle_list_sources(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize, Default)]
        struct Req {
            #[serde(default)]
            user_email_hint: Option<String>,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params)).unwrap_or_default();
        to_json(read_rpc::list_sources_rpc(&config, req.user_email_hint).await?)
    })
}

pub(super) fn handle_search(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        struct Req {
            query: String,
            k: u32,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params))?;
        to_json(read_rpc::search_rpc(&config, req.query, req.k).await?)
    })
}

pub(super) fn handle_recall(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        struct Req {
            query: String,
            k: u32,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params))?;
        to_json(read_rpc::recall_rpc(&config, req.query, req.k).await?)
    })
}

pub(super) fn handle_entity_index_for(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        struct Req {
            chunk_id: String,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params))?;
        to_json(read_rpc::entity_index_for_rpc(&config, req.chunk_id).await?)
    })
}

pub(super) fn handle_chunks_for_entity(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        struct Req {
            entity_id: String,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params))?;
        to_json(read_rpc::chunks_for_entity_rpc(&config, req.entity_id).await?)
    })
}

pub(super) fn handle_top_entities(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        struct Req {
            #[serde(default)]
            kind: Option<String>,
            limit: u32,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params))?;
        to_json(read_rpc::top_entities_rpc(&config, req.kind, req.limit).await?)
    })
}

pub(super) fn handle_chunk_score(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        struct Req {
            chunk_id: String,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params))?;
        to_json(read_rpc::chunk_score_rpc(&config, req.chunk_id).await?)
    })
}

pub(super) fn handle_delete_chunk(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        struct Req {
            chunk_id: String,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params))?;
        to_json(read_rpc::delete_chunk_rpc(&config, req.chunk_id).await?)
    })
}

pub(super) fn handle_delete_source(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        struct Req {
            source_id: String,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params))?;
        to_json(read_rpc::delete_source_rpc(&config, req.source_id).await?)
    })
}

pub(super) fn handle_graph_export(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize, Default)]
        struct Req {
            #[serde(default)]
            mode: Option<read_rpc::GraphMode>,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params)).unwrap_or_default();
        to_json(read_rpc::graph_export_rpc(&config, req.mode.unwrap_or_default()).await?)
    })
}

pub(super) fn handle_obsidian_vault_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize, Default)]
        struct Req {
            #[serde(default)]
            obsidian_config_dir: Option<String>,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params)).unwrap_or_default();
        to_json(read_rpc::obsidian_vault_status_rpc(&config, req.obsidian_config_dir).await?)
    })
}

pub(super) fn handle_vault_health_check(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize, Default)]
        struct Req {
            #[serde(default)]
            obsidian_config_dir: Option<String>,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params)).unwrap_or_default();
        to_json(read_rpc::vault_health_check_rpc(&config, req.obsidian_config_dir).await?)
    })
}

pub(super) fn handle_flush_source(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        #[derive(serde::Deserialize)]
        struct Req {
            source_scope: String,
        }
        let config = config_rpc::load_config_with_timeout().await?;
        let req = parse_value::<Req>(Value::Object(params))?;
        to_json(read_rpc::flush_source_tree_rpc(&config, &req.source_scope).await?)
    })
}

pub(super) fn handle_flush_now(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(read_rpc::flush_now_rpc(&config).await?)
    })
}

pub(super) fn handle_wipe_all(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(read_rpc::wipe_all_rpc(&config).await?)
    })
}

pub(super) fn handle_reset_tree(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(read_rpc::reset_tree_rpc(&config).await?)
    })
}

// ── Pipeline / control handlers ───────────────────────────────────────────

pub(super) fn handle_pipeline_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(rpc::pipeline_status_rpc(&config).await?)
    })
}

pub(super) fn handle_set_enabled(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::SetEnabledRequest>(Value::Object(params))?;
        let mut config = config_rpc::load_config_with_timeout().await?;
        to_json(rpc::set_enabled_rpc(&mut config, req).await?)
    })
}

pub(super) fn handle_smart_walk(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        use crate::openhuman::memory::chat::build_chat_provider;
        use crate::openhuman::memory::query::smart_walk::{
            run_smart_walk, SmartWalkOptions, SmartWalkStopReason,
        };

        #[derive(serde::Deserialize)]
        struct Req {
            query: String,
            #[serde(default = "default_namespace")]
            namespace: String,
            #[serde(default)]
            max_turns: Option<u64>,
            #[serde(default)]
            model: Option<String>,
        }
        fn default_namespace() -> String {
            "default".into()
        }

        let req = parse_value::<Req>(Value::Object(params))?;
        let config = config_rpc::load_config_with_timeout().await?;

        let chat_provider = build_chat_provider(&config)
            .map_err(|e| format!("smart_walk: build chat provider failed: {e}"))?;

        struct Adapter {
            inner: std::sync::Arc<dyn crate::openhuman::memory::chat::ChatProvider>,
        }

        #[async_trait::async_trait]
        impl crate::openhuman::inference::provider::traits::Provider for Adapter {
            async fn chat_with_system(
                &self,
                system: Option<&str>,
                message: &str,
                _model: &str,
                temperature: f64,
            ) -> anyhow::Result<String> {
                let prompt = crate::openhuman::memory::chat::ChatPrompt {
                    system: system.unwrap_or("").to_string(),
                    user: message.to_string(),
                    temperature,
                    kind: "memory_smart_walk_rpc",
                };
                self.inner.chat_for_text(&prompt).await
            }

            async fn chat_with_history(
                &self,
                messages: &[crate::openhuman::inference::provider::traits::ChatMessage],
                model: &str,
                temperature: f64,
            ) -> anyhow::Result<String> {
                let system = messages
                    .iter()
                    .find(|m| m.role == "system")
                    .map(|m| m.content.as_str());
                let user: String = messages
                    .iter()
                    .filter(|m| m.role != "system")
                    .map(|m| m.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                self.chat_with_system(system, &user, model, temperature)
                    .await
            }
        }

        let adapter = Adapter {
            inner: chat_provider,
        };

        let opts = SmartWalkOptions {
            max_turns: req.max_turns.map(|n| n as usize).unwrap_or(12),
            namespace: req.namespace,
            model: req.model,
            content_root: None,
        };

        let outcome = run_smart_walk(&config, &adapter, &req.query, opts)
            .await
            .map_err(|e| format!("smart_walk error: {e}"))?;

        let stopped = match outcome.stopped_reason {
            SmartWalkStopReason::Answered => "answered",
            SmartWalkStopReason::MaxTurnsReached => "max_turns",
            SmartWalkStopReason::LlmGaveUp => "llm_gave_up",
            SmartWalkStopReason::Error(_) => "error",
        };

        let result = serde_json::json!({
            "answer": outcome.answer,
            "turns_used": outcome.turns_used,
            "evidence_count": outcome.evidence.len(),
            "stopped_reason": stopped,
            "evidence": outcome.evidence.iter().map(|e| serde_json::json!({
                "source_path": e.source_path,
                "snippet": e.snippet,
                "relevance": e.relevance,
            })).collect::<Vec<_>>(),
            "trace": outcome.trace.iter().map(|s| serde_json::json!({
                "turn": s.turn,
                "action": s.action,
                "args_summary": s.args_summary,
                "result_preview": s.result_preview,
            })).collect::<Vec<_>>(),
        });
        to_json(RpcOutcome::new(result, vec![]))
    })
}

pub(super) fn handle_doctor(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(rpc::doctor_rpc(&config).await?)
    })
}

pub(super) fn handle_retry_failed(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(rpc::retry_failed_rpc(&config).await?)
    })
}

// ── Shared helpers ────────────────────────────────────────────────────────

pub(super) fn parse_value<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("invalid params: {e}"))
}

pub(super) fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
