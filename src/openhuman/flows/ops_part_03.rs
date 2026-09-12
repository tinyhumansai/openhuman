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
static INFERENCE_PROBE_CACHE: LazyLock<std::sync::Mutex<InferenceProbeCacheMap>> =
    LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Invalidate every cached Layer-2 probe result. Checked defensively on every
/// call so a signed-out session (whether the initial one or a later
/// account-switch) can never serve a stale cached "ready" — the moment
/// `is_signed_out` flips true the next successful probe starts a fresh TTL
/// window. Clears the whole cache rather than just the current key: a
/// sign-out is a session-wide event, not scoped to one role.
fn invalidate_inference_probe_cache_if_signed_out() {
    if crate::openhuman::cron::scheduler_gate::is_signed_out() {
        INFERENCE_PROBE_CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }
}

async fn cached_probe_inference_readiness(role: &str, config: &Config) -> Result<(), String> {
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

    let result =
        crate::openhuman::inference::provider::probe_inference_readiness(role, config).await;
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
///    [`role_for_model_tier`](crate::openhuman::inference::provider::role_for_model_tier).
/// 2. A static (non-`=`) `agent_ref` whose custom
///    [`AgentRegistryEntry`](crate::openhuman::agent::registry::AgentRegistryEntry)
///    itself pins a `model` (e.g. `hint:reasoning`) — resolved the same way
///    [`OpenHumanAgentRunner::run_via_harness`](crate::openhuman::flows::tinyflows::caps::OpenHumanAgentRunner)
///    does via `resolve_node_model(&request, entry_model)`, using the same
///    sync, config-only accessor
///    ([`find_custom_in_config`](crate::openhuman::agent::registry::find_custom_in_config))
///    it calls.
/// 3. Otherwise, caps.rs's own default role (`"summarization"`, its fallback
///    absent a `role` field on the completion request).
///
/// A static `agent_ref` that instead resolves to a shipped/TOML harness
/// `AgentDefinition` (`AgentRoute::Harness`) can *also* pin a model via
/// `ModelSpec::Exact`/`ModelSpec::Hint` — but `ModelSpec::Inherit` (the
/// default) resolves against the *parent* agent's live model at spawn time,
/// which this static, pre-run gate has no parent turn to read. Resolving only
/// the Exact/Hint cases here — while silently mis-defaulting every
/// `Inherit`-using definition — would be a half-correct, fragile lookup, so
/// this case falls back to the default role rather than guess.
/// TODO(B45): resolve agent_ref-pinned model for harness `AgentDefinition`s
/// once a parent-model-free resolution path exists.
fn agent_node_role(config: &Config, node: &tinyflows::model::Node) -> &'static str {
    let pinned_model = node
        .config
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(model) = pinned_model {
        return crate::openhuman::inference::provider::role_for_model_tier(model);
    }

    let static_agent_ref = node
        .config
        .get("agent_ref")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('='));
    if let Some(agent_ref) = static_agent_ref {
        if let Some(entry_model) =
            crate::openhuman::agent::registry::find_custom_in_config(config, agent_ref)
                .and_then(|entry| entry.model)
        {
            let entry_model = entry_model.trim();
            if !entry_model.is_empty() {
                return crate::openhuman::inference::provider::role_for_model_tier(entry_model);
            }
        }
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
struct InferenceReadinessEvaluation {
    /// One of `"ready"`, `"signed_out"`, `"provider_not_configured"`, `"error"`
    /// — the fixed vocabulary shared with the proposal payload.
    status: &'static str,
    /// User-actionable prose; `None` only when `status == "ready"`.
    message: Option<String>,
    /// The offending node id, when applicable (absent for `"ready"`).
    node_id: Option<String>,
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
async fn evaluate_inference_readiness(
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
        crate::openhuman::inference::provider::factory::resolves_to_managed_backend(
            agent_node_role(config, node),
            config,
        )
    });
    let needs_session =
        crate::openhuman::inference::provider::factory::current_host_requires_session()
            || needs_backend_session;

    // Layer 1: signed-out is the cheapest, most decisive check. Session-wide
    // — checked once for the whole graph, not per node/role.
    if needs_session && crate::openhuman::cron::scheduler_gate::is_signed_out() {
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
    let session_result = if needs_backend_session {
        crate::openhuman::inference::provider::factory::verify_backend_session_active(config)
    } else {
        crate::openhuman::inference::provider::factory::verify_session_active(config)
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

// ─────────────────────────────────────────────────────────────────────────────
// Tool-contract enforcement gate (systemic tool-contract fix, Part 2)
// ─────────────────────────────────────────────────────────────────────────────
//
// `validate_binding_resolvability` (above) statically proves a binding's
// SHAPE is sound (envelope dereference, agent output schema). It has no
// opinion on whether a `tool_call` node's `slug` is a REAL Composio action,
// or whether the args it wires cover that action's REAL required set — a
// builder could pass a hallucinated slug (`SLACK_POST_MESSAGE_TO_CHANNEL`,
// which 404s at runtime) or omit a genuinely required arg, and
// `validate_binding_resolvability` would have nothing to say about either.
// [`validate_tool_contracts`] is that missing HARD gate, grounded in
// [`crate::openhuman::flows::tinyflows::caps::fetch_live_toolkit_catalog`] — the
// FULL LIVE Composio catalog, not the static curated subset.

/// Statically proves every `tool_call` node's `config.slug` is a REAL action
/// in the LIVE Composio catalog for its toolkit, and that every one of that
/// action's REAL required args is present (non-null) in `config.args` —
/// rejecting the graph (a non-empty `Vec` = reject; empty = pass) when
/// either check fails. Wired into `propose_workflow` / `revise_workflow` /
/// `save_workflow` alongside [`validate_binding_resolvability`].
///
/// Skipped for a `slug` that is `=`-derived (resolved from upstream/trigger
/// data at runtime — nothing to check statically) or a native `oh:` tool (no
/// Composio contract at all).
///
/// **Best-effort on catalog availability, not on catalog CONTENT**: when the
/// live-catalog fetch itself fails (no backend session, network error) the
/// node is SKIPPED with a debug log — never rejected — because a
/// hallucinated slug can only be confirmed hallucinated once the real
/// catalog was actually reachable; `graph_wiring_warnings`'s
/// `composio_required_args` checks share this exact contract. Once the
/// catalog IS reachable, though, both checks below are HARD: an unreal slug
/// or a missing required arg rejects the graph outright, unlike the
/// advisory output-field/`split_out.path` WARNs in `graph_wiring_warnings`
/// (Part 2c/2d) — those degrade gracefully because a binding to an unknown
/// field can't be proven wrong, whereas a nonexistent slug or a missing
/// required arg are both provably broken.
/// Whether OpenHuman ships a STATIC curated catalog for `toolkit`. This is the
/// exact condition both [`validate_tool_contracts`]'s curation gate and
/// `tinyflows::caps::flow_tool_allowed`'s runtime Path A use to decide a toolkit
/// is a hard curated-only allowlist: for such a toolkit a real-but-uncurated
/// action is rejected on EVERY real run, so the author-time gate and the early
/// builder-tool warnings (`get_tool_contract` / `search_tool_catalog`) must all
/// agree on it — one home for the check so they cannot drift.
pub(crate) fn toolkit_has_curated_catalog(toolkit: &str) -> bool {
    // The curated catalogs moved to `tinymemory-bus` (OpenHuman#5560) and are
    // reachable directly via `catalog_for_toolkit`, so this no longer needs
    // the engine-backed provider-registry hop the comment here used to
    // explain: `get_provider(toolkit).curated_tools()` was verified identical
    // to `catalog_for_toolkit(toolkit)` for every toolkit that had one, and
    // tinymemory v1.13.4 deleted the registry outright, so the hop is gone
    // rather than merely unnecessary.
    use crate::openhuman::integrations::composio::providers::catalog_for_toolkit;
    catalog_for_toolkit(toolkit).is_some()
}

pub(crate) async fn validate_tool_contracts(config: &Config, graph: &WorkflowGraph) -> Vec<String> {
    use crate::openhuman::flows::tinyflows::caps::{
        fetch_live_toolkit_catalog, missing_required_args, unsupported_arg_names,
    };
    use tinymemory_api::composio::toolkit_from_slug;

    let mut errors = Vec::new();
    for node in &graph.nodes {
        if node.kind != NodeKind::ToolCall {
            continue;
        }
        let Some(slug) = node.config.get("slug").and_then(Value::as_str) else {
            continue;
        };
        // `=`-derived slugs resolve from upstream/trigger data at runtime —
        // nothing to check statically. Native `oh:` tools have no Composio
        // contract.
        if slug.starts_with('=') || slug.starts_with("oh:") {
            continue;
        }
        let Some(toolkit) = toolkit_from_slug(slug) else {
            continue;
        };
        let Some(catalog) = fetch_live_toolkit_catalog(config, &toolkit).await else {
            tracing::debug!(
                target: "flows",
                node = %node.id,
                %slug,
                %toolkit,
                "[flows] tool-contract check: live catalog fetch failed — skipping (best-effort, never false-rejects)"
            );
            continue;
        };

        let Some(contract) = catalog.iter().find(|c| c.slug.eq_ignore_ascii_case(slug)) else {
            tracing::warn!(
                target: "flows",
                node = %node.id,
                %slug,
                %toolkit,
                "[flows] tool-contract check: slug is not a real action in the live catalog — rejecting"
            );
            errors.push(format!(
                "Node '{}': `{slug}` is not a real action in the `{toolkit}` toolkit's live \
                 Composio catalog — use search_tool_catalog {{ query: ..., toolkit: \"{toolkit}\" \
                 }} to find a real action slug.",
                node.id
            ));
            continue;
        };

        // Mirror `flow_tool_allowed`'s Path A: a toolkit OpenHuman ships a
        // static curated catalog for is a hard curated-only allowlist at
        // RUNTIME — `find_curated` rejects any slug that isn't one of the
        // curated actions, regardless of whether it's a real live action.
        // `search_tool_catalog`/`get_tool_contract` deliberately surface
        // real-but-uncurated actions too (ranking signal only, never
        // hidden — see `ToolContract::is_curated`'s doc), so without this
        // check a graph could pass authoring/save with a real-but-uncurated
        // action on a curated toolkit and then fail every run with "tool
        // not permitted". Hold authoring to the same bar the runtime gate
        // enforces instead of loosening the runtime gate.
        let has_static_catalog = toolkit_has_curated_catalog(&toolkit);
        if has_static_catalog && !contract.is_curated {
            tracing::warn!(
                target: "flows",
                node = %node.id,
                %slug,
                %toolkit,
                "[flows] tool-contract check: slug is real but not curated for a statically-catalogued toolkit — rejecting to match the runtime allowlist"
            );
            errors.push(format!(
                "Node '{}': `{slug}` is a real `{toolkit}` action but not one of OpenHuman's \
                 curated actions for `{toolkit}` — the runtime tool gate only allows curated \
                 actions for toolkits with a curated catalog, so this would be rejected on \
                 every run. Use search_tool_catalog {{ query: ..., toolkit: \"{toolkit}\" }} and \
                 pick a result with `featured: true`.",
                node.id
            ));
            continue;
        }

        let args = node.config.get("args").cloned().unwrap_or(Value::Null);
        let missing = missing_required_args(&contract.required_args, &args);
        if !missing.is_empty() {
            tracing::warn!(
                target: "flows",
                node = %node.id,
                %slug,
                ?missing,
                "[flows] tool-contract check: required arg(s) missing or null — rejecting"
            );
            let list = missing
                .iter()
                .map(|m| format!("`{m}`"))
                .collect::<Vec<_>>()
                .join(", ");
            errors.push(format!(
                "Node '{}': tool_call `{slug}` is missing required arg(s) {list} — wire each \
                 from an upstream node's output, e.g. \"{}\": \
                 \"=nodes.<node_id>.item.json.<field>\" (call get_tool_contract {{ slug: \
                 \"{slug}\" }} for the exact required_args list).",
                node.id, missing[0]
            ));
        }

        // [B13] Arg-NAME validity: `missing_required_args` only proves a
        // required arg is PRESENT — it says nothing about whether every arg
        // the builder wired is actually a property this action's schema
        // recognizes. A misnamed/unsupported field (the live bug: wiring
        // `SLACK_SEND_MESSAGE` with `text` when the action wants
        // `markdown_text`) sails through the check above unrejected — a
        // value IS present, just under the wrong key — and only surfaces as
        // a runtime 400 from the real provider. `unsupported_arg_names`
        // returns `None` when the schema can't be used to validate names
        // (unknown schema, or `additionalProperties: true`) — that case is
        // deliberately never rejected here (best-effort, same posture as the
        // rest of this gate).
        if let Some(unsupported) = unsupported_arg_names(contract.input_schema.as_ref(), &args) {
            if !unsupported.is_empty() {
                let valid_names: Vec<String> = contract
                    .input_schema
                    .as_ref()
                    .and_then(|s| s.get("properties"))
                    .and_then(Value::as_object)
                    .map(|props| {
                        let mut names: Vec<String> = props.keys().cloned().collect();
                        names.sort();
                        names
                    })
                    .unwrap_or_default();
                tracing::warn!(
                    target: "flows",
                    node = %node.id,
                    %slug,
                    ?unsupported,
                    ?valid_names,
                    "[flows] tool-contract check: arg name(s) not declared by the action's \
                     input schema — rejecting"
                );
                let bad_list = unsupported
                    .iter()
                    .map(|m| format!("`{m}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let valid_suffix = if valid_names.is_empty() {
                    String::new()
                } else {
                    format!(
                        " — valid arg names for `{slug}` are: {}",
                        valid_names.join(", ")
                    )
                };
                errors.push(format!(
                    "Node '{}': tool_call `{slug}` has unsupported arg name(s) {bad_list} — not \
                     a property of this action's input schema{valid_suffix}. Call \
                     get_tool_contract {{ slug: \"{slug}\" }} and use the exact property names \
                     from `input_schema` (never guess an arg name).",
                    node.id
                ));
            }
        }
    }
    errors
}

// ─────────────────────────────────────────────────────────────────────────────
// Connection-ref gate (WS3): a Composio tool_call's `connection_ref` must name
// a real connected account of the RIGHT toolkit
// ─────────────────────────────────────────────────────────────────────────────
//
// Transcript audit: the user's connections were `twitter →
// composio:twitter:ca_JX6QU88UfSk4`, `gmail → composio:gmail:ca_vX_WA8FsqNmE`,
// `tiktok → composio:tiktok:ca_LPCp3WQpaDma`. The agent wired
// `composio:twitter:ca_LPCp3WQpaDma` and `composio:gmail:ca_LPCp3WQpaDma` (the
// TIKTOK id) onto the Twitter and Gmail tool_call nodes. dry_run / validate /
// propose all returned ok:true — nothing cross-checked the id against the user's
// real connections, nor the ref's toolkit segment against the slug — and it
// would fail on the first real run. This gate closes that gap: it parses the
// ref, enforces the toolkit segment matches the slug (needs no I/O), and — when
// the live connection list is reachable — that the id names a real connected
// account of that toolkit, naming the correct ref when it can.

/// Parses a `composio:<toolkit>:<id>` connection_ref into its `(toolkit, id)`
/// segments. Mirrors [`crate::openhuman::flows::tinyflows::caps::composio_connection_id`]'s
/// rsplit for the id (everything after the LAST `:`), taking everything between
/// the `composio:` prefix and that last `:` as the toolkit. Returns `None` for
/// anything that isn't this shape (missing `composio:` prefix, no `:` after it,
/// or an empty toolkit/id segment).
fn parse_composio_connection_ref(conn_ref: &str) -> Option<(&str, &str)> {
    let rest = conn_ref.strip_prefix("composio:")?;
    let (toolkit, id) = rest.rsplit_once(':')?;
    if toolkit.trim().is_empty() || id.trim().is_empty() {
        return None;
    }
    Some((toolkit.trim(), id.trim()))
}

/// First connected account `connection_ref` for `toolkit` (case-insensitive)
/// from `conns`, used to name the correct ref in a rejection's "did you mean"
/// hint. `None` when the toolkit has no connection at all.
fn first_connection_ref_for_toolkit(conns: &[FlowConnection], toolkit: &str) -> Option<String> {
    conns
        .iter()
        .find(|c| {
            c.toolkit
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(toolkit))
        })
        .map(|c| c.connection_ref.clone())
}
