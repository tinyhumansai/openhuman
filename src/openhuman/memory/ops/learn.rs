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
    // config (and the local_ai.enabled guard) for an empty batch.
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

    if !config.local_ai.enabled {
        return Err("memory_learn_all requires local_ai.enabled=true".to_string());
    }

    let mut results = Vec::with_capacity(target_ns.len());
    for namespace in &target_ns {
        tracing::info!(
            "[memory.learn] running summarization for namespace='{}'",
            namespace
        );
        let outcome =
            crate::openhuman::tree_summarizer::ops::tree_summarizer_run(&config, namespace).await;
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
