use serde::Deserialize;

use crate::core_server::helpers::parse_params;
use crate::core_server::types::InvocationResult;

pub async fn try_dispatch(
    method: &str,
    params: serde_json::Value,
) -> Option<Result<InvocationResult, String>> {
    match method {
        "ai.list_memory_files" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct ListMemoryFilesParams {
                    relative_dir: Option<String>,
                }

                let payload: ListMemoryFilesParams = parse_params(params)?;
                let relative_dir = payload.relative_dir.unwrap_or_else(|| "memory".to_string());
                let files = crate::ai::sessions::ai_list_memory_files(relative_dir).await?;
                InvocationResult::ok(files)
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
                let content =
                    crate::ai::sessions::ai_read_memory_file(payload.relative_path).await?;
                InvocationResult::ok(content)
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
                let wrote = crate::ai::sessions::ai_write_memory_file(
                    payload.relative_path,
                    payload.content,
                )
                .await?;
                InvocationResult::ok(wrote)
            }
            .await,
        ),

        "ai.memory_init" => Some(
            async move {
                let initialized = crate::ai::ai_memory_init().await?;
                InvocationResult::ok(initialized)
            }
            .await,
        ),

        "ai.memory_get_file" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct MemoryGetFileParams {
                    path: String,
                }

                let payload: MemoryGetFileParams = parse_params(params)?;
                let file = crate::ai::ai_memory_get_file(payload.path).await?;
                InvocationResult::ok(file)
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
                let deleted = crate::ai::ai_memory_delete_chunks_by_path(payload.path).await?;
                InvocationResult::ok(deleted)
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
                let upserted = crate::ai::ai_memory_upsert_chunk(payload.chunk).await?;
                InvocationResult::ok(upserted)
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
                let upserted = crate::ai::ai_memory_upsert_file(payload.file).await?;
                InvocationResult::ok(upserted)
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
                let set = crate::ai::ai_memory_set_meta(payload.key, payload.value).await?;
                InvocationResult::ok(set)
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
                let value = crate::ai::ai_memory_get_meta(payload.key).await?;
                InvocationResult::ok(value)
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
                let results =
                    crate::ai::ai_memory_fts_search(payload.query, payload.limit).await?;
                InvocationResult::ok(results)
            }
            .await,
        ),

        "ai.memory_get_all_embeddings" => Some(
            async move {
                let embeddings = crate::ai::ai_memory_get_all_embeddings().await?;
                InvocationResult::ok(embeddings)
            }
            .await,
        ),

        "ai.sessions_init" => Some(
            async move {
                let initialized = crate::ai::sessions::ai_sessions_init().await?;
                InvocationResult::ok(initialized)
            }
            .await,
        ),

        "ai.sessions_load_index" => Some(
            async move {
                let index = crate::ai::sessions::ai_sessions_load_index().await?;
                InvocationResult::ok(index)
            }
            .await,
        ),

        "ai.sessions_update_index" => Some(
            async move {
                #[derive(Debug, Deserialize)]
                struct UpdateSessionIndexParams {
                    session_id: String,
                    entry: crate::ai::sessions::SessionIndexEntry,
                }

                let payload: UpdateSessionIndexParams = parse_params(params)?;
                let updated = crate::ai::sessions::ai_sessions_update_index(
                    payload.session_id,
                    payload.entry,
                )
                .await?;
                InvocationResult::ok(updated)
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
                let appended = crate::ai::sessions::ai_sessions_append_transcript(
                    payload.session_id,
                    payload.line,
                )
                .await?;
                InvocationResult::ok(appended)
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
                let lines =
                    crate::ai::sessions::ai_sessions_read_transcript(payload.session_id).await?;
                InvocationResult::ok(lines)
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
                let deleted =
                    crate::ai::sessions::ai_sessions_delete(payload.session_id).await?;
                InvocationResult::ok(deleted)
            }
            .await,
        ),

        "ai.sessions_list" => Some(
            async move {
                let sessions = crate::ai::sessions::ai_sessions_list().await?;
                InvocationResult::ok(sessions)
            }
            .await,
        ),

        _ => None,
    }
}
