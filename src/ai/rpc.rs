use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

fn parse_params<T: DeserializeOwned>(params: serde_json::Value) -> Result<T, String> {
    serde_json::from_value(params).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: Serialize>(value: T) -> Result<serde_json::Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

pub async fn try_dispatch(
    method: &str,
    params: serde_json::Value,
) -> Option<Result<serde_json::Value, String>> {
    match method {
        "ai.list_memory_files" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct ListMemoryFilesParams {
                    relative_dir: Option<String>,
                }

                let payload: ListMemoryFilesParams = parse_params(params)?;
                let relative_dir = payload.relative_dir.unwrap_or_else(|| "memory".to_string());
                to_json(crate::ai::sessions::ai_list_memory_files(relative_dir).await?)
            }
            .await,
        ),

        "ai.read_memory_file" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct ReadMemoryFileParams {
                    relative_path: String,
                }

                let payload: ReadMemoryFileParams = parse_params(params)?;
                to_json(crate::ai::sessions::ai_read_memory_file(payload.relative_path).await?)
            }
            .await,
        ),

        "ai.write_memory_file" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct WriteMemoryFileParams {
                    relative_path: String,
                    content: String,
                }

                let payload: WriteMemoryFileParams = parse_params(params)?;
                to_json(
                    crate::ai::sessions::ai_write_memory_file(
                        payload.relative_path,
                        payload.content,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "ai.memory_init" => Some(async move { to_json(crate::ai::ai_memory_init().await?) }.await),

        "ai.memory_get_file" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct MemoryGetFileParams {
                    path: String,
                }

                let payload: MemoryGetFileParams = parse_params(params)?;
                to_json(crate::ai::ai_memory_get_file(payload.path).await?)
            }
            .await,
        ),

        "ai.memory_delete_chunks_by_path" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct MemoryDeleteChunksByPathParams {
                    path: String,
                }

                let payload: MemoryDeleteChunksByPathParams = parse_params(params)?;
                to_json(crate::ai::ai_memory_delete_chunks_by_path(payload.path).await?)
            }
            .await,
        ),

        "ai.memory_upsert_chunk" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct MemoryUpsertChunkParams {
                    chunk: crate::ai::ChunkRecord,
                }

                let payload: MemoryUpsertChunkParams = parse_params(params)?;
                to_json(crate::ai::ai_memory_upsert_chunk(payload.chunk).await?)
            }
            .await,
        ),

        "ai.memory_upsert_file" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct MemoryUpsertFileParams {
                    file: crate::ai::FileRecord,
                }

                let payload: MemoryUpsertFileParams = parse_params(params)?;
                to_json(crate::ai::ai_memory_upsert_file(payload.file).await?)
            }
            .await,
        ),

        "ai.memory_set_meta" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct MemorySetMetaParams {
                    key: String,
                    value: String,
                }

                let payload: MemorySetMetaParams = parse_params(params)?;
                to_json(crate::ai::ai_memory_set_meta(payload.key, payload.value).await?)
            }
            .await,
        ),

        "ai.memory_get_meta" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct MemoryGetMetaParams {
                    key: String,
                }

                let payload: MemoryGetMetaParams = parse_params(params)?;
                to_json(crate::ai::ai_memory_get_meta(payload.key).await?)
            }
            .await,
        ),

        "ai.memory_fts_search" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct MemoryFtsSearchParams {
                    query: String,
                    limit: i64,
                }

                let payload: MemoryFtsSearchParams = parse_params(params)?;
                to_json(crate::ai::ai_memory_fts_search(payload.query, payload.limit).await?)
            }
            .await,
        ),

        "ai.memory_get_all_embeddings" => {
            Some(async move { to_json(crate::ai::ai_memory_get_all_embeddings().await?) }.await)
        }

        "ai.memory_get_chunks" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct MemoryGetChunksParams {
                    path: String,
                }

                let payload: MemoryGetChunksParams = parse_params(params)?;
                to_json(crate::ai::ai_memory_get_chunks(payload.path).await?)
            }
            .await,
        ),

        "ai.memory_cache_embedding" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct MemoryCacheEmbeddingParams {
                    entry: crate::ai::EmbeddingCacheEntry,
                }

                let payload: MemoryCacheEmbeddingParams = parse_params(params)?;
                to_json(crate::ai::ai_memory_cache_embedding(payload.entry).await?)
            }
            .await,
        ),

        "ai.memory_get_cached_embedding" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct MemoryGetCachedEmbeddingParams {
                    provider: String,
                    model: String,
                    hash: String,
                }

                let payload: MemoryGetCachedEmbeddingParams = parse_params(params)?;
                to_json(
                    crate::ai::ai_memory_get_cached_embedding(
                        payload.provider,
                        payload.model,
                        payload.hash,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "ai.sessions_init" => {
            Some(async move { to_json(crate::ai::sessions::ai_sessions_init().await?) }.await)
        }

        "ai.sessions_load_index" => {
            Some(async move { to_json(crate::ai::sessions::ai_sessions_load_index().await?) }.await)
        }

        "ai.sessions_update_index" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct UpdateSessionIndexParams {
                    session_id: String,
                    entry: crate::ai::sessions::SessionIndexEntry,
                }

                let payload: UpdateSessionIndexParams = parse_params(params)?;
                to_json(
                    crate::ai::sessions::ai_sessions_update_index(
                        payload.session_id,
                        payload.entry,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "ai.sessions_append_transcript" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct AppendTranscriptParams {
                    session_id: String,
                    line: String,
                }

                let payload: AppendTranscriptParams = parse_params(params)?;
                to_json(
                    crate::ai::sessions::ai_sessions_append_transcript(
                        payload.session_id,
                        payload.line,
                    )
                    .await?,
                )
            }
            .await,
        ),

        "ai.sessions_read_transcript" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct ReadTranscriptParams {
                    session_id: String,
                }

                let payload: ReadTranscriptParams = parse_params(params)?;
                to_json(crate::ai::sessions::ai_sessions_read_transcript(payload.session_id).await?)
            }
            .await,
        ),

        "ai.sessions_delete" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct DeleteSessionParams {
                    session_id: String,
                }

                let payload: DeleteSessionParams = parse_params(params)?;
                to_json(crate::ai::sessions::ai_sessions_delete(payload.session_id).await?)
            }
            .await,
        ),

        "ai.sessions_list" => {
            Some(async move { to_json(crate::ai::sessions::ai_sessions_list().await?) }.await)
        }

        _ => None,
    }
}
