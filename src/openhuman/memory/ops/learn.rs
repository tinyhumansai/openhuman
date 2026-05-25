//! `memory_learn_all` — runs the tree summarizer over namespaces sequentially.

use std::collections::BTreeSet;

use crate::rpc::RpcOutcome;

use super::helpers::active_memory_client;

/// Per-namespace outcome for `memory_learn_all`.
#[derive(Debug, serde::Serialize)]
pub struct NamespaceLearnResult {
    pub namespace: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Result returned by `memory_learn_all`.
#[derive(Debug, serde::Serialize)]
pub struct LearnAllResult {
    pub namespaces_processed: usize,
    pub results: Vec<NamespaceLearnResult>,
}

/// Parameters for `memory_learn_all`.
#[derive(Debug, serde::Deserialize)]
pub struct LearnAllParams {
    /// Optional list of namespaces to constrain. Defaults to all namespaces.
    #[serde(default)]
    pub namespaces: Option<Vec<String>>,
}

/// Run the tree summarizer over all (or a constrained set of) namespaces.
///
/// Enumerates namespaces via `namespace_list`, then for each runs
/// `tree_summarizer_run`. Results are collected per-namespace; a failing
/// namespace does not abort the rest. Runs sequentially to avoid saturating
/// the local AI provider.
pub async fn memory_learn_all(
    params: LearnAllParams,
) -> Result<RpcOutcome<LearnAllResult>, String> {
    tracing::info!(
        "[memory.learn] memory_learn_all: entry namespaces={:?}",
        params.namespaces
    );

    // Resolve the target namespace list.
    let client = active_memory_client().await?;
    let all_ns = client.list_namespaces().await?;
    tracing::debug!("[memory.learn] available namespaces: {:?}", all_ns);

    let target_ns: Vec<String> = match &params.namespaces {
        Some(requested) if !requested.is_empty() => {
            let mut seen = BTreeSet::new();
            let filtered: Vec<_> = requested
                .iter()
                .filter(|ns| all_ns.contains(ns))
                .filter(|ns| seen.insert((*ns).clone()))
                .cloned()
                .collect();
            tracing::debug!("[memory.learn] constrained to namespaces: {:?}", filtered);
            filtered
        }
        Some(requested) => {
            // Explicit empty list → no-op (don't fall back to all namespaces).
            let mut seen = BTreeSet::new();
            let filtered: Vec<_> = requested
                .iter()
                .filter(|ns| all_ns.contains(ns))
                .filter(|ns| seen.insert((*ns).clone()))
                .cloned()
                .collect();
            tracing::debug!(
                "[memory.learn] Some([]) empty request → no-op or filtered to {:?}",
                filtered
            );
            filtered
        }
        None => {
            tracing::debug!("[memory.learn] using all {} namespaces", all_ns.len());
            all_ns
        }
    };

    // Short-circuit when there are no namespaces to process — avoids loading
    // config (and the local_ai.runtime_enabled guard) for an empty batch.
    if target_ns.is_empty() {
        tracing::info!(
            "[memory.learn] memory_learn_all: no namespaces to process, returning early"
        );
        return Ok(RpcOutcome::new(
            LearnAllResult {
                namespaces_processed: 0,
                results: vec![],
            },
            vec![],
        ));
    }

    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e}"))?;

    if !config.local_ai.runtime_enabled {
        return Err("memory_learn_all requires local_ai.runtime_enabled=true".to_string());
    }

    let mut results = Vec::with_capacity(target_ns.len());
    for namespace in &target_ns {
        tracing::info!(
            "[memory.learn] running summarization for namespace='{}'",
            namespace
        );
        let outcome = crate::openhuman::memory_tree::tree_runtime::ops::tree_summarizer_run(
            &config, namespace,
        )
        .await;
        match outcome {
            Ok(_) => {
                tracing::info!("[memory.learn] namespace='{}' ok", namespace);
                results.push(NamespaceLearnResult {
                    namespace: namespace.clone(),
                    status: "ok".to_string(),
                    error: None,
                });
            }
            Err(e) => {
                tracing::warn!("[memory.learn] namespace='{}' error: {}", namespace, e);
                results.push(NamespaceLearnResult {
                    namespace: namespace.clone(),
                    status: "error".to_string(),
                    error: Some(e),
                });
            }
        }
    }

    let namespaces_processed = results.len();
    tracing::info!(
        "[memory.learn] memory_learn_all: done processed={} results={:?}",
        namespaces_processed,
        results
            .iter()
            .map(|r| (&r.namespace, &r.status))
            .collect::<Vec<_>>()
    );
    Ok(RpcOutcome::new(
        LearnAllResult {
            namespaces_processed,
            results,
        },
        vec![],
    ))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::OnceLock;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::openhuman::memory_store::NamespaceDocumentInput;

    fn ensure_memory_client() {
        static WORKSPACE: OnceLock<PathBuf> = OnceLock::new();
        let workspace = WORKSPACE.get_or_init(|| {
            let tmp = TempDir::new().expect("tempdir");
            let path = tmp.path().join("workspace");
            std::fs::create_dir_all(&path).expect("workspace dir");
            std::mem::forget(tmp);
            path
        });
        let _ = crate::openhuman::memory::global::init(workspace.clone());
    }

    struct WorkspaceEnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        previous: Option<OsString>,
    }

    impl WorkspaceEnvGuard {
        fn set(path: &std::path::Path) -> Self {
            let lock = crate::openhuman::config::TEST_ENV_LOCK.lock().unwrap();
            let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
            std::env::set_var("OPENHUMAN_WORKSPACE", path);
            Self {
                _lock: lock,
                previous,
            }
        }
    }

    impl Drop for WorkspaceEnvGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var("OPENHUMAN_WORKSPACE", previous);
            } else {
                std::env::remove_var("OPENHUMAN_WORKSPACE");
            }
        }
    }

    async fn seed_namespace(prefix: &str) -> String {
        ensure_memory_client();
        let namespace = format!("{prefix}-{}", uuid::Uuid::new_v4());
        let client = crate::openhuman::memory::global::client().expect("memory client");
        client
            .put_doc_light(NamespaceDocumentInput {
                namespace: namespace.clone(),
                key: format!("key-{}", uuid::Uuid::new_v4()),
                title: "Test".into(),
                content: "Seed content".into(),
                source_type: "doc".into(),
                priority: "normal".into(),
                tags: vec!["test".into()],
                metadata: json!({"source": "test"}),
                category: "core".into(),
                session_id: None,
                document_id: None,
            })
            .await
            .expect("seed namespace doc");
        namespace
    }

    async fn write_config_with_runtime_enabled(
        workspace_root: &std::path::Path,
        runtime_enabled: bool,
    ) {
        let _guard = WorkspaceEnvGuard::set(workspace_root);
        let mut config = crate::openhuman::config::Config::load_or_init()
            .await
            .expect("load config");
        config.local_ai.runtime_enabled = runtime_enabled;
        config.save().await.expect("save config");
    }

    #[tokio::test]
    async fn memory_learn_all_is_noop_for_explicit_empty_namespace_list() {
        ensure_memory_client();
        let outcome = memory_learn_all(LearnAllParams {
            namespaces: Some(vec![]),
        })
        .await
        .expect("empty list should early-return");
        assert_eq!(outcome.value.namespaces_processed, 0);
        assert!(outcome.value.results.is_empty());
        assert!(outcome.logs.is_empty());
    }

    #[tokio::test]
    async fn memory_learn_all_is_noop_when_requested_namespaces_do_not_exist() {
        ensure_memory_client();
        let missing = format!("missing-{}", uuid::Uuid::new_v4());
        let outcome = memory_learn_all(LearnAllParams {
            namespaces: Some(vec![missing]),
        })
        .await
        .expect("unknown namespaces should filter to no-op");
        assert_eq!(outcome.value.namespaces_processed, 0);
        assert!(outcome.value.results.is_empty());
    }

    #[tokio::test]
    async fn memory_learn_all_filters_missing_namespaces_and_dedupes_requested_order() {
        let namespace_a = seed_namespace("memory-learn-a").await;
        let namespace_b = seed_namespace("memory-learn-b").await;
        let missing = format!("missing-{}", uuid::Uuid::new_v4());
        let tmp = TempDir::new().expect("tempdir");
        write_config_with_runtime_enabled(tmp.path(), true).await;
        let _workspace = WorkspaceEnvGuard::set(tmp.path());

        let outcome = memory_learn_all(LearnAllParams {
            namespaces: Some(vec![
                missing,
                namespace_b.clone(),
                namespace_a.clone(),
                namespace_b.clone(),
            ]),
        })
        .await
        .expect("existing namespaces with runtime enabled should run");

        assert_eq!(outcome.value.namespaces_processed, 2);
        assert_eq!(outcome.value.results.len(), 2);
        assert_eq!(outcome.value.results[0].namespace, namespace_b);
        assert_eq!(outcome.value.results[1].namespace, namespace_a);
        assert!(outcome.value.results.iter().all(|r| r.status == "ok"));
        assert!(outcome.value.results.iter().all(|r| r.error.is_none()));
    }

    #[tokio::test]
    async fn memory_learn_all_requires_local_ai_once_existing_namespace_is_selected() {
        let namespace = seed_namespace("memory-learn-runtime").await;
        let tmp = TempDir::new().expect("tempdir");
        write_config_with_runtime_enabled(tmp.path(), false).await;
        let _workspace = WorkspaceEnvGuard::set(tmp.path());

        let err = memory_learn_all(LearnAllParams {
            namespaces: Some(vec![namespace]),
        })
        .await
        .expect_err("runtime-disabled config should hard-fail");

        assert!(err.contains("memory_learn_all requires local_ai.runtime_enabled=true"));
    }

    #[tokio::test]
    async fn memory_learn_all_uses_all_namespaces_when_none_is_requested() {
        let namespace_a = seed_namespace("memory-learn-all-a").await;
        let namespace_b = seed_namespace("memory-learn-all-b").await;
        let tmp = TempDir::new().expect("tempdir");
        write_config_with_runtime_enabled(tmp.path(), true).await;
        let _workspace = WorkspaceEnvGuard::set(tmp.path());

        let outcome = memory_learn_all(LearnAllParams { namespaces: None })
            .await
            .expect("runtime-enabled config should process all namespaces");

        assert!(
            outcome.value.namespaces_processed >= 2,
            "expected at least the two seeded namespaces to be processed"
        );
        let namespaces: std::collections::BTreeSet<_> = outcome
            .value
            .results
            .iter()
            .map(|r| r.namespace.as_str())
            .collect();
        assert!(namespaces.contains(namespace_a.as_str()));
        assert!(namespaces.contains(namespace_b.as_str()));
        assert!(outcome
            .value
            .results
            .iter()
            .filter(|r| r.namespace == namespace_a || r.namespace == namespace_b)
            .all(|r| r.status == "ok" && r.error.is_none()));
    }
}
