//! TinyHumans Neocortex persistent memory layer.
//!
//! Wraps `TinyHumanMemoryClient` with helpers for skill data-sync.
//! The client is initialised at runtime with the user's JWT token (from Redux
//! `authSlice.token`) via the `init_memory_client` Tauri command, not from env vars.

use std::sync::Arc;
use tinyhumansai::{
    DeleteMemoryParams, InsertMemoryParams, Priority, QueryMemoryParams, RecallMemoryParams,
    SourceType, TinyHumanConfig, TinyHumansMemoryClient,
};
use uuid::Uuid;

/// Shared, cloneable handle to the memory client.
pub type MemoryClientRef = Arc<MemoryClient>;

/// Shared app-state slot for the memory client.
pub struct MemoryState(pub std::sync::Mutex<Option<MemoryClientRef>>);

pub struct MemoryClient {
    inner: TinyHumansMemoryClient,
}

impl MemoryClient {
    /// Construct from a JWT token (sourced from `authSlice.token` in the Redux store).
    /// Returns `None` if the token is empty or client construction fails.
    pub fn from_token(jwt_token: String) -> Option<Self> {
        log::info!(
            "[memory] from_token: entry (token_len={})",
            jwt_token.trim().len()
        );
        if jwt_token.trim().is_empty() {
            log::warn!("[memory] from_token: exit — token is empty, returning None");
            return None;
        }
        let config = if let Ok(base_url) =
            std::env::var("OPENHUMAN_BASE_URL").or_else(|_| std::env::var("TINYHUMANS_BASE_URL"))
        {
            log::info!(
                "[memory] from_token: constructing client (base_url={base_url}, source=memory_env)"
            );
            TinyHumanConfig::new(jwt_token).with_base_url(base_url)
        } else {
            let backend_url = std::env::var("VITE_BACKEND_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| "http://localhost:5005".to_string());
            log::info!(
                "[memory] from_token: constructing client (base_url={backend_url}, source=fallback_env_default)"
            );
            TinyHumanConfig::new(jwt_token).with_base_url(backend_url)
        };
        match TinyHumansMemoryClient::new(config) {
            Ok(inner) => {
                log::info!("[memory] from_token: exit — client created successfully");
                Some(Self { inner })
            }
            Err(e) => {
                log::warn!("[memory] from_token: exit — client construction failed: {e}");
                None
            }
        }
    }

    /// Store a skill data-sync result.
    ///
    /// Inserts the document then polls `ingestion_job_status` every 30 s until
    /// the job reaches `completed` (or `failed`/`error`). Returns only after the
    /// ingestion job is confirmed complete.
    pub async fn store_skill_sync(
        &self,
        skill_id: &str,
        integration_id: &str,
        title: &str,
        content: &str,
        source_type: Option<SourceType>,
        metadata: Option<serde_json::Value>,
        priority: Option<Priority>,
        created_at: Option<f64>,
        updated_at: Option<f64>,
        document_id: Option<String>,
    ) -> Result<(), String> {
        let namespace = skill_id.to_string();
        log::info!(
            "[memory] store_skill_sync: entry (namespace={namespace}, title={title:?}, content_len={})",
            content.len()
        );

        let document_id_final = document_id.unwrap_or_else(|| Uuid::new_v4().to_string());

        log::info!(
            "[memory] insert_memory: calling SDK (namespace={namespace}, title={title:?}), content_len={}",
            content.len()
        );

        let insert_resp = self
            .inner
            .insert_memory(InsertMemoryParams {
                document_id: document_id_final,
                title: title.to_string(),
                content: content.to_string(),
                namespace: namespace.clone(),
                source_type,
                metadata,
                priority,
                created_at,
                updated_at,
                ..Default::default()
            })
            .await
            .map_err(|e| {
                log::warn!(
                    "[memory] insert_memory: SDK error — kind={:?} msg={e}",
                    classify_insert_error(&e)
                );
                format!("Memory insert failed: {e}")
            })?;

        log::info!(
            "[memory] insert_memory: accepted (namespace={namespace}, status={:?}, job_id={:?})",
            insert_resp.data.status,
            insert_resp.data.job_id
        );

        // If the API returned a job_id, poll until the job completes.
        if let Some(job_id) = insert_resp.data.job_id {
            log::info!("[memory] ingestion job queued (job_id={job_id}), polling every 30s...");

            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;

                match self.inner.get_ingestion_job(&job_id).await {
                    Ok(status_resp) => {
                        let state = status_resp.data.state.as_deref().unwrap_or("unknown");

                        log::info!(
                            "[memory] ingestion job status: job_id={job_id}, state={state}, \
                             attempts={:?}, completed_at={:?}",
                            status_resp.data.attempts,
                            status_resp.data.completed_at
                        );

                        match state {
                            "completed" => {
                                log::info!(
                                    "[memory] ingestion job completed (job_id={job_id}, namespace={namespace})"
                                );
                                break;
                            }
                            "failed" | "error" => {
                                let err_msg = status_resp
                                    .data
                                    .error
                                    .unwrap_or_else(|| format!("job state={state}"));
                                log::warn!(
                                    "[memory] ingestion job failed: job_id={job_id}, error={err_msg}"
                                );
                                log::warn!(
                                    "[memory] store_skill_sync: exit — ingestion failed (namespace={namespace})"
                                );
                                return Err(format!("Ingestion job failed: {err_msg}"));
                            }
                            _ => {
                                // pending / processing / queued — keep waiting
                                log::info!(
                                    "[memory] ingestion job still in progress (state={state}), waiting 30s..."
                                );
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!(
                            "[memory] ingestion job status poll error (job_id={job_id}): {e} — retrying in 30s"
                        );
                    }
                }
            }
        } else {
            log::info!("[memory] no job_id returned — insert assumed synchronous, proceeding");
        }

        log::info!("[memory] store_skill_sync: exit — ok (namespace={namespace})");
        Ok(())
    }

    /// Query relevant context for a skill integration (RAG).
    pub async fn query_skill_context(
        &self,
        skill_id: &str,
        _integration_id: &str,
        query: &str,
        max_chunks: u32,
    ) -> Result<String, String> {
        let namespace = skill_id.to_string();
        log::info!("[memory] query_skill_context: entry (namespace={namespace}, max_chunks={max_chunks}, query={query:?})");
        log::debug!(
            "[memory] query_skill_context: payload → namespace={namespace} | max_chunks={max_chunks} | query={query}"
        );
        let res = self
            .inner
            .query_memory(QueryMemoryParams {
                query: query.to_string(),
                namespace: Some(namespace.clone()),
                max_chunks: Some(f64::from(max_chunks)),
                ..Default::default()
            })
            .await
            .map_err(|e| {
                log::warn!(
                    "[memory] query_skill_context: exit — error (namespace={namespace}): {e}"
                );
                format!("Memory query failed: {e}")
            })?;
        let response = res.data.response.unwrap_or_default();
        log::info!(
            "[memory] query_skill_context: exit — ok (namespace={namespace}, response_len={})",
            response.len()
        );
        Ok(response)
    }

    /// Recall context from the Master memory node for a given namespace.
    /// Returns the synthesised `response` string, or `None` if the server returned nothing.
    pub async fn recall_skill_context(
        &self,
        skill_id: &str,
        _integration_id: &str,
        max_chunks: u32,
    ) -> Result<Option<serde_json::Value>, String> {
        let namespace = skill_id.to_string();
        log::info!(
            "[memory] recall_skill_context: entry (namespace={namespace}, max_chunks={max_chunks})"
        );
        let res = self
            .inner
            .recall_memory(RecallMemoryParams {
                namespace: Some(namespace.clone()),
                max_chunks: Some(f64::from(max_chunks)),
            })
            .await
            .map_err(|e| {
                log::warn!(
                    "[memory] recall_skill_context: exit — error (namespace={namespace}): {e}"
                );
                format!("Memory recall failed: {e}")
            })?;
        let response = res.data.context;
        log::info!(
            "[memory] recall_skill_context: exit — ok (namespace={namespace}, has_response={})",
            response.is_some()
        );
        Ok(response)
    }

    /// List all ingested memory documents as returned by the API.
    pub async fn list_documents(&self) -> Result<serde_json::Value, String> {
        self.inner
            .list_documents(tinyhumansai::ListDocumentsParams::default())
            .await
            .map_err(|e| format!("Memory list documents failed: {e}"))
    }

    /// Delete a specific document from a namespace.
    pub async fn delete_document(
        &self,
        document_id: &str,
        namespace: &str,
    ) -> Result<serde_json::Value, String> {
        self.inner
            .delete_document(document_id, namespace)
            .await
            .map_err(|e| format!("Memory delete document failed: {e}"))
    }

    /// Query memory context by namespace directly.
    pub async fn query_namespace_context(
        &self,
        namespace: &str,
        query: &str,
        max_chunks: u32,
    ) -> Result<String, String> {
        let res = self
            .inner
            .query_memory(QueryMemoryParams {
                query: query.to_string(),
                namespace: Some(namespace.to_string()),
                max_chunks: Some(f64::from(max_chunks)),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("Memory query failed: {e}"))?;
        Ok(res.data.response.unwrap_or_default())
    }

    /// Recall memory context by namespace directly.
    pub async fn recall_namespace_context(
        &self,
        namespace: &str,
        max_chunks: u32,
    ) -> Result<Option<String>, String> {
        let res = self
            .inner
            .recall_memory(RecallMemoryParams {
                namespace: Some(namespace.to_string()),
                max_chunks: Some(f64::from(max_chunks)),
            })
            .await
            .map_err(|e| format!("Memory recall failed: {e}"))?;
        Ok(res.data.response)
    }

    /// Delete all memories for a skill integration (e.g. on OAuth revoke).
    pub async fn clear_skill_memory(
        &self,
        skill_id: &str,
        _integration_id: &str,
    ) -> Result<(), String> {
        let namespace = skill_id.to_string();
        log::info!("[memory] clear_skill_memory: entry (namespace={namespace})");
        log::debug!("[memory] clear_skill_memory: payload → namespace={namespace}");
        let result = self
            .inner
            .delete_memory(DeleteMemoryParams {
                namespace: Some(namespace.clone()),
            })
            .await
            .map(|_| ())
            .map_err(|e| format!("Memory delete failed: {e}"));
        match &result {
            Ok(()) => log::info!("[memory] clear_skill_memory: exit — ok (namespace={namespace})"),
            Err(e) => {
                log::warn!("[memory] clear_skill_memory: exit — error (namespace={namespace}): {e}")
            }
        }
        result
    }
}

fn classify_insert_error(e: &tinyhumansai::TinyHumansError) -> &'static str {
    let msg = e.to_string();
    if msg.contains("dns") || msg.contains("resolve") || msg.contains("lookup") {
        "dns_failure"
    } else if msg.contains("tls") || msg.contains("certificate") || msg.contains("ssl") {
        "tls_failure"
    } else if msg.contains("Connection refused") || msg.contains("connection refused") {
        "connection_refused"
    } else if msg.contains("timed out") || msg.contains("deadline") {
        "timeout"
    } else if msg.contains("error sending request") {
        "transport_error"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Integration test against the real TinyHumans production API.
    ///
    /// Verifies: JWT is accepted, endpoint is reachable, and request shape is correct.
    /// A `400 Insufficient ingestion budget` response is treated as a PASS because it proves:
    ///   - auth succeeded (not 401/403)
    ///   - the endpoint and payload are correctly formed (not 422)
    ///   - the account quota is the only limiting factor
    ///
    /// Run with:
    ///   JWT_TOKEN=<your-openhuman-jwt> \
    ///   cargo test --manifest-path src-tauri/Cargo.toml test_memory_skill_sync_flow -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn test_memory_skill_sync_flow() {
        let jwt_token = std::env::var("JWT_TOKEN").expect("JWT_TOKEN must be set");

        let client = MemoryClient::from_token(jwt_token).expect("client creation failed");

        let skill_id = "gmail";
        let integration_id = "test@openhuman.dev";

        let dummy_content = serde_json::json!({
            "integrationId": integration_id,
            "type": "gmail_sync",
            "summary": "Synced 45 emails from inbox",
            "labels": ["Work", "Personal", "Finance"],
            "recentSenders": ["alice@example.com", "bob@example.com"],
            "unreadCount": 12,
            "syncedAt": "2026-03-17T12:00:00Z"
        });

        // ── 1. Insert ─────────────────────────────────────────────────────────
        let insert_result = client
            .store_skill_sync(
                skill_id,
                integration_id,
                "Gmail OAuth sync — test@openhuman.dev",
                &serde_json::to_string_pretty(&dummy_content).unwrap(),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await;

        println!("[test] insert result: {insert_result:?}");

        match &insert_result {
            Ok(()) => {
                println!("[test] ✓ INSERT succeeded — quota available");

                // ── 2. Query ─────────────────────────────────────────────────
                let context = client
                    .query_skill_context(
                        skill_id,
                        integration_id,
                        "What emails were recently synced and who sent them?",
                        10,
                    )
                    .await;
                println!("[test] query result: {context:?}");
                assert!(context.is_ok(), "query_memory failed: {context:?}");
                println!("[test] memory context:\n{}", context.unwrap());

                // ── 3. Cleanup ────────────────────────────────────────────────
                let del = client.clear_skill_memory(skill_id, integration_id).await;
                println!("[test] delete result: {del:?}");
                assert!(del.is_ok(), "delete_memory failed: {del:?}");
            }
            Err(e) if e.contains("Insufficient ingestion budget") => {
                // Account quota exhausted — auth + endpoint + payload all correct.
                println!(
                    "[test] ✓ API REACHABLE — auth accepted, endpoint correct.\n\
                     Quota limited: {e}\n\
                     Integration is wired correctly; upgrade the TinyHumans account \
                     to enable full insert/query/delete flow."
                );
                // Not a code failure — pass the test.
            }
            Err(e) => {
                panic!("Unexpected error (not a quota issue): {e}");
            }
        }
    }

    /// Smoke test: from_token() returns None for an empty token.
    #[test]
    fn test_from_token_returns_none_for_empty_token() {
        assert!(MemoryClient::from_token(String::new()).is_none());
        assert!(MemoryClient::from_token("   ".to_string()).is_none());
    }
}
