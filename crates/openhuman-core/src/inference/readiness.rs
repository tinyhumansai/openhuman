//! Product-facing inference readiness policy shared by workflow authoring and execution.
//!
//! Callers provide neutral node identifiers and resolved workload roles. This
//! module owns session checks, provider probes, caching, and structured failure
//! classification; host domains retain graph interpretation and user-facing
//! message formatting.

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use std::collections::{BTreeMap, HashMap};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const PROBE_CACHE_TTL: Duration = Duration::from_secs(60);
type CacheKey = (String, std::path::PathBuf);
type CacheEntry = (Instant, Result<(), String>);
type Cache = HashMap<CacheKey, CacheEntry>;

/// A readiness failure grouped by resolved workload role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadinessFailure {
    pub status: &'static str,
    pub session_wide: bool,
    pub role: &'static str,
    pub message: String,
    pub node_ids: Vec<String>,
}

/// Structured result for a workload that uses inference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadinessEvaluation {
    /// `ready`, `signed_out`, `provider_not_configured`, or `error`.
    pub status: &'static str,
    pub failures: Vec<ReadinessFailure>,
}

static PROBE_CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn invalidate_cache_if_signed_out() {
    if crate::platform::llm_gate::is_signed_out() {
        PROBE_CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }
}

/// Probe one role, caching both definitive success and failure for one minute.
pub async fn cached_probe(ctx: &DispatchCtx, role: &str, config: &Config) -> Result<(), String> {
    invalidate_cache_if_signed_out();
    let key = (role.to_string(), config.config_path.clone());
    if let Some((checked_at, result)) = PROBE_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
        .cloned()
    {
        if checked_at.elapsed() < PROBE_CACHE_TTL {
            tracing::debug!(target: "inference", role, cached_ready = result.is_ok(), "reusing cached inference-readiness probe");
            return result;
        }
    }
    let result = crate::inference::provider::probe_inference_readiness(ctx, role, config).await;
    PROBE_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, (Instant::now(), result.clone()));
    result
}

fn classify(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();
    if lower.contains("session_expired") || lower.contains("sign in") {
        "signed_out"
    } else if lower.contains("api key not configured") {
        "provider_not_configured"
    } else {
        "error"
    }
}

/// Evaluate readiness for neutral `(node id, workload role)` inputs.
///
/// An empty input returns `None`, allowing non-inference workloads to skip the
/// check. Probe failures are grouped and ordered by role for deterministic
/// results. Transient probe errors are already fail-open in the provider layer.
pub async fn evaluate(
    ctx: &DispatchCtx,
    config: &Config,
    nodes: &[(String, &'static str)],
) -> Option<ReadinessEvaluation> {
    let (first_node_id, _) = nodes.first()?;
    let needs_backend_session = nodes.iter().any(|(_, role)| {
        crate::inference::provider::factory::resolves_to_managed_backend(role, config)
    });
    let needs_session = needs_backend_session
        || (crate::inference::provider::factory::access_gates::current_host_requires_session()
            && nodes.iter().any(|(_, role)| {
                let provider = crate::inference::provider::factory::provider_for_role(role, config);
                !crate::inference::provider::factory::access_gates::provider_uses_independent_auth(
                    &provider,
                )
            }));

    if needs_session && crate::platform::llm_gate::is_signed_out() {
        tracing::debug!(target: "inference", node = %first_node_id, "signed out; rejecting inference workload");
        return Some(ReadinessEvaluation {
            status: "signed_out",
            failures: vec![ReadinessFailure {
                status: "signed_out",
                session_wide: true,
                role: nodes[0].1,
                message: "Inference unavailable: you are signed out. Sign in to OpenHuman to run agent nodes.".into(),
                node_ids: vec![first_node_id.clone()],
            }],
        });
    }

    // Unit-test configs intentionally lack a persisted app-session JWT. Keep
    // the production check identical while allowing isolated policy tests.
    #[cfg(not(test))]
    let session_result = if !needs_session {
        Ok(())
    } else if needs_backend_session {
        crate::inference::provider::factory::access_gates::verify_backend_session_active(config)
            .await
    } else {
        crate::inference::provider::factory::access_gates::verify_session_active(config).await
    };
    #[cfg(not(test))]
    if let Err(error) = session_result {
        tracing::debug!(target: "inference", node = %first_node_id, %error, "no active inference session");
        return Some(ReadinessEvaluation {
            status: "signed_out",
            failures: vec![ReadinessFailure {
                status: "signed_out",
                session_wide: true,
                role: nodes[0].1,
                message: format!(
                    "Inference unavailable: {error} Sign in to OpenHuman to run agent nodes."
                ),
                node_ids: vec![first_node_id.clone()],
            }],
        });
    }

    let mut nodes_by_role: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    for (node_id, role) in nodes {
        nodes_by_role.entry(role).or_default().push(node_id.clone());
    }
    let mut failures = Vec::new();
    for (role, node_ids) in nodes_by_role {
        tracing::debug!(target: "inference", nodes = ?node_ids, role, "probing inference readiness");
        if let Err(message) = cached_probe(ctx, role, config).await {
            tracing::warn!(target: "inference", nodes = ?node_ids, role, "inference readiness probe rejected: {message}");
            failures.push(ReadinessFailure {
                status: classify(&message),
                session_wide: false,
                role,
                message,
                node_ids,
            });
        }
    }
    if failures.is_empty() {
        return Some(ReadinessEvaluation {
            status: "ready",
            failures,
        });
    }
    let statuses: Vec<_> = failures
        .iter()
        .map(|failure| classify(&failure.message))
        .collect();
    let status = if statuses.contains(&"signed_out") {
        "signed_out"
    } else if statuses.contains(&"provider_not_configured") {
        "provider_not_configured"
    } else {
        "error"
    };
    Some(ReadinessEvaluation { status, failures })
}

#[cfg(test)]
#[path = "readiness_tests.rs"]
mod tests;
