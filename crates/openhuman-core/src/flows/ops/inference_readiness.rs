use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// Inference-readiness check: provider-connectivity (issue B45)
// ─────────────────────────────────────────────────────────────────────────────
//
// An `agent` node's completion (`OpenHumanLlm::complete` in
// `tinyflows/caps.rs`) resolves a chat model exactly like every other
// inference caller in this host — but no check previously inspected that
// resolution at all. `compute_required_connections` only walks `tool_call`
// Composio nodes; an `agent` node's own hard dependency, a working LLM
// provider, went completely unchecked. The confirmed failure: a signed-in
// user whose managed-backend account has no provider API key configured gets
// an HTTP 400 `{"success":false,"error":"API key not configured for
// provider","errorCode":"BAD_REQUEST"}` — but only mid-run, wrapped several
// layers deep as `capability error: graph error: capability error: model
// error: ...`.
//
// **Design correction (judge finding on live run 104aab90 — see git log for
// the full writeup):** this was originally wired in as a HARD author gate
// (`run_builder_gates`), rejecting `propose_workflow`/`edit_workflow`
// outright. In practice that meant a graph whose only problem was "the user
// hasn't configured a provider yet" could never be proposed at all — the
// copilot detected `provider_not_configured`, tried to propose anyway, was
// blocked, and trailed off with no workflow shown to the user. The correct
// placement is:
//
// - **Author time (`build_builder_proposal`)** — ADVISORY ONLY. Authoring
//   always succeeds; `evaluate_inference_readiness`'s result rides along on
//   the proposal payload as `inference_status`/`inference_message` so the UI
//   can render a "connect your provider" nudge next to the built workflow.
// - **Run time (`run_flow_body`)** — HARD gate. A real run (never
//   `dry_run_workflow`, which is a sandbox) checks readiness before invoking
//   the tinyflows engine and fails the run row cleanly with an actionable
//   message if the graph's agent node(s) can't currently reach a provider —
//   see `validate_inference_readiness`'s call site in `run_flow_body`.
//
// Two layers, cheapest and most decisive first:
//
// - **Layer 1 (sync)** — the desktop session itself: signed out
//   (`scheduler_gate::is_signed_out`), or no valid `app-session` JWT
//   (`inference::provider::factory::verify_session_active`, the exact check
//   every custom-provider construction already gates on).
// - **Layer 2 (async, cached)** — one cheap real probe per DISTINCT resolved
//   role (`inference::provider::probe_inference_readiness`) to catch the
//   "signed in but no provider API key configured for this account" class of
//   failure that Layer 1 cannot see. A graph can mix agent nodes pinned to
//   different models (e.g. one `hint:reasoning`, one plain `chat`) that route
//   to different provider configs — each distinct role is probed once, not
//   once per node, and every probe's result caches BOTH a successful and a
//   definitively-negative result for a short TTL — a propose → edit → save →
//   run authoring/run burst hits the network at most once per role per TTL
//   window, whichever way the probe comes back. This is safe to cache
//   negative because `probe_inference_readiness` (and, beneath it,
//   `OpenHumanBackendModel::probe_readiness`) already fails OPEN (`Ok(())`)
//   on anything transient — a timeout, a transport error, a 5xx — so an
//   `Err` reaching this cache is always the definitive, config-level "not
//   ready" signal, never a flake that a naive cache would freeze in place.
//
// [`evaluate_inference_readiness`] is the single evaluation both
// [`validate_inference_readiness`] (the hard gate) and
// [`build_builder_proposal`]'s `inference_status` payload field consume, so
// the gate and the UI-facing status can never disagree.

/// Cache TTL for the Layer-2 managed-backend/role probe.
const INFERENCE_PROBE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// Cache key: (workload role, session identity). `config.config_path` stands
/// in for "session identity" — within one desktop process there is exactly
/// one active config/session, so this is stable in production, while
/// distinct `Config`s (as every test builds its own `tempfile` workspace)
/// naturally get distinct cache entries instead of bleeding a cached result
/// from one test/session into an unrelated one. Keying on `role` alone would
/// NOT be enough: two different sessions (or two tests) can both resolve the
/// literal role `"summarization"` to entirely different, unrelated outcomes.
type InferenceProbeCacheKey = (String, std::path::PathBuf);
/// A cached probe outcome: when it was taken, and the definitive result.
type InferenceProbeCacheEntry = (std::time::Instant, Result<(), String>);
/// The probe cache map, factored out to keep the `static` type readable
/// (clippy::type-complexity).
type InferenceProbeCacheMap =
    std::collections::HashMap<InferenceProbeCacheKey, InferenceProbeCacheEntry>;

/// Process-global cache of Layer-2 probe outcomes, keyed by
/// [`InferenceProbeCacheKey`]. Both `Ok` and `Err` entries are served from
/// cache within [`INFERENCE_PROBE_CACHE_TTL`] (design correction, B45 —
/// previously only `Ok` was cached, so a signed-in-but-unconfigured account
/// re-hit the network on every one of `edit_workflow` / `validate_workflow` /
/// `propose_workflow` / a run's own preflight in a single authoring turn — up
/// to 4 network round trips observed in one live judge-flagged turn). A
/// cached `Err` is still only ever the definitive class (see the module doc
/// above on fail-open) — a fixed provider becomes visible again at most
/// `INFERENCE_PROBE_CACHE_TTL` later, or immediately on sign-out/back-in via
/// [`invalidate_inference_probe_cache_if_signed_out`].
pub(super) static INFERENCE_PROBE_CACHE: LazyLock<std::sync::Mutex<InferenceProbeCacheMap>> =
    LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Invalidate every cached Layer-2 probe result. Checked defensively on every
/// call so a signed-out session (whether the initial one or a later
/// account-switch) can never serve a stale cached "ready" — the moment
/// `is_signed_out` flips true the next successful probe starts a fresh TTL
/// window. Clears the whole cache rather than just the current key: a
/// sign-out is a session-wide event, not scoped to one role.
fn invalidate_inference_probe_cache_if_signed_out() {
    if crate::cron::scheduler_gate::is_signed_out() {
        INFERENCE_PROBE_CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }
}

pub(super) async fn cached_probe_inference_readiness(
    role: &str,
    config: &Config,
) -> Result<(), String> {
    invalidate_inference_probe_cache_if_signed_out();

    let key: InferenceProbeCacheKey = (role.to_string(), config.config_path.clone());

    if let Some((checked_at, result)) = INFERENCE_PROBE_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
        .cloned()
    {
        if checked_at.elapsed() < INFERENCE_PROBE_CACHE_TTL {
            tracing::debug!(
                target: "flows",
                role,
                cached_ready = result.is_ok(),
                "[flows] inference-readiness: reusing cached probe result"
            );
            return result;
        }
    }

    let result = crate::inference::provider::probe_inference_readiness(role, config).await;
    INFERENCE_PROBE_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, (std::time::Instant::now(), result.clone()));
    result
}

/// The workload role an `agent` node's completion effectively runs on —
/// mirrors the exact mapping `OpenHumanLlm::complete` (`tinyflows/caps.rs`)
/// applies, so this probe checks the same route the node will actually
/// dispatch to at run time. Precedence (findings A+B on this gate):
///
/// 1. Node `config.model` — a managed tier or `hint:*` alias, translated via
///    [`role_for_model_tier`](crate::inference::provider::role_for_model_tier).
/// 2. A static (non-`=`) `agent_ref` whose custom
///    [`AgentRegistryEntry`](crate::agent::registry::AgentRegistryEntry)
///    itself pins a `model` (e.g. `hint:reasoning`) — resolved the same way
///    [`OpenHumanAgentRunner::run_via_harness`](crate::flows::tinyflows::caps::OpenHumanAgentRunner)
///    does via `resolve_node_model(&request, entry_model)`, using the same
///    sync, config-only accessor
///    ([`find_custom_in_config`](crate::agent::registry::find_custom_in_config))
///    it calls.
/// 3. Otherwise, caps.rs's own default role (`"summarization"`, its fallback
///    absent a `role` field on the completion request).
///
/// Known harness agents use the session builder's provider-role resolver.
/// Definition hints are interpreted exactly as at construction; sub-agent
/// ModelSpec resolution is a separate runtime path and is not guessed here.
pub(super) fn agent_node_role(config: &Config, node: &tinyflows::model::Node) -> &'static str {
    if let Some(agent_ref) = node
        .config
        .get("agent_ref")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('='))
    {
        use crate::agent::harness::definition::AgentDefinitionRegistry;
        if let Err(error) = AgentDefinitionRegistry::init_global(&config.workspace_dir) {
            tracing::debug!(target: "flows", %error,
                "[flows] readiness: agent definition registry unavailable; using registry fallback");
        }
        let definition = AgentDefinitionRegistry::global().and_then(|r| r.get(agent_ref));
        let custom = crate::agent::registry::find_custom_in_config(config, agent_ref);
        if definition.is_some() || custom.is_some() {
            let entry_model = if definition.is_some() {
                None
            } else {
                custom.as_ref().and_then(|entry| entry.model.as_deref())
            };
            let override_model =
                crate::flows::tinyflows::caps::resolve_node_model(&node.config, entry_model).map(
                    |model| crate::flows::tinyflows::caps::harness_model_default_override(&model),
                );
            return crate::agent::session_host::provider_role_for_definition(
                agent_ref,
                override_model
                    .as_deref()
                    .or(config.default_model.as_deref()),
                definition,
            );
        }
    }
    let pinned_model = node
        .config
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(model) = pinned_model {
        return crate::inference::provider::role_for_model_tier(model);
    }

    "summarization"
}

/// Classifies an inference-readiness failure message into the fixed wire
/// vocabulary `build_builder_proposal`'s `inference_status` payload and this
/// gate's prose both use (`"signed_out" | "provider_not_configured" |
/// "error"`).
///
/// Defensive ordering: a message that still smells like a dead session (an
/// unlikely race between this gate's own signed-out check and the async
/// probe) is classified `signed_out` before the more specific
/// `provider_not_configured` pattern; anything else falls back to the generic
/// `error` bucket (a BYOK-incomplete config, an unknown provider slug, a
/// local-only privacy-mode block, …) rather than mislabeling it as a
/// provider-key problem.
fn classify_inference_error_message(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();
    if lower.contains("session_expired") || lower.contains("sign in") {
        "signed_out"
    } else if lower.contains("api key not configured") {
        "provider_not_configured"
    } else {
        "error"
    }
}

/// Outcome of [`evaluate_inference_readiness`] for a graph that has at least
/// one applicable `agent` node.
pub(super) struct InferenceReadinessEvaluation {
    /// One of `"ready"`, `"signed_out"`, `"provider_not_configured"`, `"error"`
    /// — the fixed vocabulary shared with the proposal payload.
    pub(super) status: &'static str,
    /// User-actionable prose; `None` only when `status == "ready"`.
    pub(super) message: Option<String>,
    /// The offending node id, when applicable (absent for `"ready"`).
    pub(super) node_id: Option<String>,
}

/// Evaluate the B45 provider-connectivity gate for `graph`.
///
/// Returns `None` when the graph has no `agent` node at all — a tool_call-only
/// graph never pays this check's cost. A dynamic `=`-derived `agent_ref` node
/// is still in scope (finding C): its concrete route is not knowable
/// statically, so its exact per-model role can't be resolved, but the node
/// still means "this graph runs inference" — it stays in scope for Layer 1
/// (signed-out/session) and gets a default-role Layer 2 probe. Only the
/// per-model role resolution is skipped for such a node, never the whole
/// check.
///
/// Every DISTINCT role across the graph's applicable `agent` nodes is probed
/// (findings A+B): Layer 1 (signed-out/session) runs once for the whole
/// graph — every agent node shares one backend session — then Layer 2 runs
/// once per distinct role (via [`cached_probe_inference_readiness`], so a
/// role already probed elsewhere in this process within the TTL is served
/// from cache). `status`/`message` report `provider_not_configured`/`error`
/// if ANY role's probe fails, naming every offending node and role.
pub(super) async fn evaluate_inference_readiness(
    config: &Config,
    graph: &WorkflowGraph,
) -> Option<InferenceReadinessEvaluation> {
    let agent_nodes: Vec<&tinyflows::model::Node> = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Agent)
        .collect();

    let first_node = *agent_nodes.first()?;
    let needs_backend_session = agent_nodes.iter().any(|node| {
        crate::inference::provider::factory::resolves_to_managed_backend(
            agent_node_role(config, node),
            config,
        )
    });
    let needs_session = needs_backend_session
        || (crate::inference::provider::factory::current_host_requires_session()
            && agent_nodes.iter().any(|node| {
                let provider = crate::inference::provider::factory::provider_for_role(
                    agent_node_role(config, node),
                    config,
                );
                !crate::inference::provider::factory::access_gates::provider_uses_independent_auth(
                    &provider,
                )
            }));

    // Layer 1: signed-out is the cheapest, most decisive check. Session-wide
    // — checked once for the whole graph, not per node/role.
    if needs_session && crate::cron::scheduler_gate::is_signed_out() {
        tracing::debug!(
            target: "flows",
            node = %first_node.id,
            "[flows] inference-readiness: signed out — rejecting"
        );
        return Some(InferenceReadinessEvaluation {
            status: "signed_out",
            message: Some(
                "Inference unavailable: you are signed out. Sign in to OpenHuman to run agent \
                 nodes."
                    .to_string(),
            ),
            node_id: Some(first_node.id.clone()),
        });
    }
    // Skipped under `#[cfg(test)]`, matching every other call site of this
    // exact check (`factory.rs`'s `unresolved_chat_model_error` and friends):
    // unit-test configs use a fresh `tempfile::tempdir()` workspace with no
    // stored `app-session` JWT by design, so this would otherwise reject
    // every agent-node graph built by the hundreds of existing flows tests
    // that have nothing to do with session state. Layer 2 below still fails
    // OPEN on a construction failure caused by a genuinely missing session
    // (see `OpenHumanBackendModel::probe_readiness`'s own doc), so production
    // behavior for a real signed-out desktop user is unchanged — only the
    // (redundant, in that case) early rejection here is test-only skipped.
    #[cfg(not(test))]
    let session_result = if !needs_session {
        Ok(())
    } else if needs_backend_session {
        crate::inference::provider::factory::access_gates::verify_backend_session_active(config)
    } else {
        crate::inference::provider::factory::access_gates::verify_session_active(config)
    };
    #[cfg(not(test))]
    if let Err(e) = session_result {
        tracing::debug!(
            target: "flows",
            node = %first_node.id,
            error = %e,
            "[flows] inference-readiness: no active backend session — rejecting"
        );
        return Some(InferenceReadinessEvaluation {
            status: "signed_out",
            message: Some(format!(
                "Inference unavailable: {e} Sign in to OpenHuman to run agent nodes."
            )),
            node_id: Some(first_node.id.clone()),
        });
    }

    // Layer 2: each node's effective role, grouped so every DISTINCT role is
    // probed exactly once (a graph with several agent nodes pinning the same
    // role must not pay the network/cache-lookup cost twice). `BTreeMap` for
    // deterministic iteration/message ordering (test-friendly, and stable
    // prose across runs).
    let mut nodes_by_role: std::collections::BTreeMap<&'static str, Vec<String>> =
        std::collections::BTreeMap::new();
    for node in &agent_nodes {
        let role = agent_node_role(config, node);
        nodes_by_role.entry(role).or_default().push(node.id.clone());
    }

    let mut failures: Vec<(&'static str, String, Vec<String>)> = Vec::new();
    for (role, node_ids) in &nodes_by_role {
        tracing::debug!(
            target: "flows",
            nodes = ?node_ids,
            role,
            "[flows] inference-readiness: probing managed-backend/role readiness"
        );
        if let Err(msg) = cached_probe_inference_readiness(role, config).await {
            tracing::warn!(
                target: "flows",
                nodes = ?node_ids,
                role,
                "[flows] inference-readiness: probe rejected — {msg}"
            );
            failures.push((role, msg, node_ids.clone()));
        }
    }

    if failures.is_empty() {
        return Some(InferenceReadinessEvaluation {
            status: "ready",
            message: None,
            node_id: None,
        });
    }

    // Defensive ordering matches `classify_inference_error_message`'s own doc:
    // `signed_out` (unlikely to reach Layer 2, given the Layer 1 check above,
    // but a race is not impossible) outranks `provider_not_configured`, which
    // outranks the generic `error` bucket.
    let statuses: Vec<&'static str> = failures
        .iter()
        .map(|(_, msg, _)| classify_inference_error_message(msg))
        .collect();
    let status = if statuses.contains(&"signed_out") {
        "signed_out"
    } else if statuses.contains(&"provider_not_configured") {
        "provider_not_configured"
    } else {
        "error"
    };

    // Single failing role naming a single node: keep the original flat
    // message shape (no node-list preamble) so the existing single-node
    // contract/tests read exactly as before. Anything broader (several
    // failing roles, or one role shared by several nodes) names every
    // offending node/role explicitly, since a flat message can no longer
    // unambiguously point at "the" offending node.
    if let [(_role, msg, node_ids)] = failures.as_slice() {
        if let [node_id] = node_ids.as_slice() {
            let message = if status == "provider_not_configured" {
                format!(
                    "This flow's agent step needs a working AI provider, but the provider \
                     returned: '{msg}'. Configure your provider API key in OpenHuman Settings > \
                     Providers, then try again."
                )
            } else {
                format!("This flow's agent step needs a working AI provider: {msg}")
            };
            return Some(InferenceReadinessEvaluation {
                status,
                message: Some(message),
                node_id: Some(node_id.clone()),
            });
        }
    }

    let message = failures
        .iter()
        .map(|(role, msg, node_ids)| {
            let nodes = node_ids
                .iter()
                .map(|id| format!("'{id}'"))
                .collect::<Vec<_>>()
                .join(", ");
            let role_status = classify_inference_error_message(msg);
            if role_status == "provider_not_configured" {
                format!(
                    "Node(s) {nodes} (role `{role}`): the provider returned: '{msg}'. Configure \
                     your provider API key in OpenHuman Settings > Providers, then try again."
                )
            } else {
                format!("Node(s) {nodes} (role `{role}`): {msg}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    Some(InferenceReadinessEvaluation {
        status,
        message: Some(format!(
            "This flow has {} agent step(s) that need a working AI provider:\n\n{message}",
            failures.len()
        )),
        node_id: None,
    })
}

/// The B45 provider-connectivity check as a gate-shaped `Vec<String>`: empty
/// when the graph's `agent` node(s) (if any) can currently reach a working
/// LLM provider, otherwise the offending node's error, naming it.
///
/// **No longer wired into `run_builder_gates`** (design correction — see the
/// module doc above): authoring is never blocked by this. Its one production
/// caller is `run_flow_body`'s run-time preflight, which fails a real run
/// cleanly before the tinyflows engine executes rather than hard-blocking the
/// author from proposing/saving the graph in the first place. See the module
/// doc above for the two-layer evaluation design.
pub(crate) async fn validate_inference_readiness(
    config: &Config,
    graph: &WorkflowGraph,
) -> Vec<String> {
    let Some(evaluation) = evaluate_inference_readiness(config, graph).await else {
        return Vec::new();
    };
    if evaluation.status == "ready" {
        return Vec::new();
    }
    let message = evaluation
        .message
        .unwrap_or_else(|| "This flow's agent step needs a working AI provider.".to_string());
    match evaluation.node_id {
        Some(node_id) => vec![format!("Node '{node_id}': {message}")],
        None => vec![message],
    }
}
