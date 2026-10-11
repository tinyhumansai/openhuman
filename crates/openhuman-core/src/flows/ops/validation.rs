use super::*;

/// Runs a raw graph JSON value through `tinyflows::migrate::migrate` (upgrade
/// an older-schema definition to current), deserializes it, and rejects a
/// structurally invalid graph via `tinyflows::validate::validate` — so a bad
/// graph is caught at the door, before it's ever persisted.
///
/// `pub(crate)` (not private) so `flows::tools::ProposeWorkflowTool` (issue
/// B4 — agent-first workflow authoring) can run a candidate graph through the
/// exact same validate/migrate path `flows_create` uses below, without
/// duplicating it. The tool only calls this — never `flows_create` itself —
/// which is what keeps the "the agent can never create a flow" invariant
/// intact: this function validates and returns, it has no persistence effect.
pub(crate) fn validate_and_migrate_graph(graph_json: Value) -> Result<WorkflowGraph, String> {
    let graph = migrate_and_deserialize_graph(graph_json)?;
    tinyflows::validate::validate(&graph).map_err(|e| e.to_string())?;
    tinyflows::compat::ensure_compatible(&graph)?;
    Ok(graph)
}

/// Every engine-incompatible topology in `graph`, mapped onto this domain's
/// validation-error shape.
///
/// The classification is [`tinyflows::compat`]'s, and belongs there: which
/// fan-in shapes the engine's barrier relief can execute is a fact about the
/// engine, not about OpenHuman. This host used to carry the whole walk.
pub(crate) fn engine_compatibility_errors(
    graph: &WorkflowGraph,
) -> Vec<tinyflows_catalog::FlowValidationError> {
    tinyflows::compat::errors(graph)
        .into_iter()
        .map(to_compat_validation_error)
        .collect()
}

// The refusal code is `tinyflows::compat`'s, re-exported at `ops::` scope
// because this module's tests assert on it by name — which is the point of a
// stable code, and what keeps a rename upstream a compile error here rather
// than a silently-passing `contains`.
#[cfg(test)]
pub(crate) use tinyflows::compat::UNSUPPORTED_NESTED_CONDITIONAL_FAN_IN;

fn to_compat_validation_error(
    error: tinyflows::compat::CompatibilityError,
) -> tinyflows_catalog::FlowValidationError {
    tinyflows_catalog::FlowValidationError {
        code: error.code.to_string(),
        message: error.message,
        node_id: error.node_id,
        field: None,
    }
}

/// Host-aware compatibility check, including saved descendants that graph-only
/// validation cannot inspect. Authoring boundaries use it before persistence;
/// execution boundaries use it before compiling a root run/resume or returning
/// a resolver graph, so an unsafe descendant cannot run after earlier effects.
pub(super) fn ensure_config_aware_engine_compatible(
    config: &Config,
    graph: &WorkflowGraph,
) -> Result<(), String> {
    match config_aware_engine_compatibility_errors(config, graph)
        .into_iter()
        .next()
    {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Runs a raw graph JSON value through migration + deserialization **without**
/// the structural `validate` step, attributing a deserialization failure to the
/// member that caused it (`nodes[1]: missing field ...`). The implementation is
/// `tinyflows::migrate::deserialize_graph`; this alias keeps the host's
/// call sites (and the authoring tools' error text) unchanged.
pub(crate) use tinyflows::migrate::deserialize_graph as migrate_and_deserialize_graph;

/// Maps a portable `tinyflows` [`ValidationError`](tinyflows::error::ValidationError)
/// into the host's structured [`FlowValidationError`], carrying its stable
/// `code`, anchoring `node_id`, and human `message`. One place so the mapping
/// stays consistent across `flows_validate` and the builder gate stack.
pub(crate) fn to_flow_validation_error(
    err: &tinyflows::error::ValidationError,
) -> tinyflows_catalog::FlowValidationError {
    tinyflows_catalog::FlowValidationError {
        code: err.code().to_string(),
        message: err.to_string(),
        node_id: err.node_id().map(str::to_string),
        field: None,
    }
}

/// Checks literal `workflow_id` children reachable from an authoring candidate.
///
/// Pure graph validation can recurse through inline children, but resolving a
/// saved child requires the host store. Keep that lookup in the config-aware
/// builder gate so strict RPC and agent-authored proposals/saves cannot bless a
/// parent that is already known to fail at execution. Dynamic `=` expressions,
/// missing ids, and store failures retain their existing runtime diagnostics;
/// this gate only rejects a saved graph whose topology is demonstrably unsafe.
pub(super) fn referenced_workflow_compatibility_errors(
    config: &Config,
    graph: &WorkflowGraph,
) -> Vec<String> {
    // The walk (depth budget, cycle/depth bookkeeping, error text) is
    // `tinyflows::compat`'s; only the saved-workflow lookup is the host's. A
    // missing flow or a store failure resolves to `None` and keeps its runtime
    // diagnostic.
    tinyflows::compat::referenced_workflow_errors(graph, &|workflow_id| {
        load_flow_graph(config, workflow_id).ok().flatten()
    })
}

/// Returns the complete engine-topology gate for a graph in its host context.
/// The graph-only half covers inline descendants; the config-aware half follows
/// literal saved-workflow references. Authoring and execution boundaries share
/// this helper so neither can accept a graph the other must reject.
pub(crate) fn config_aware_engine_compatibility_errors(
    config: &Config,
    graph: &WorkflowGraph,
) -> Vec<String> {
    let direct = engine_compatibility_errors(graph);
    if !direct.is_empty() {
        return direct
            .into_iter()
            .map(|error| format!("{}: {}", error.code, error.message))
            .collect();
    }
    referenced_workflow_compatibility_errors(config, graph)
}

/// Validates a candidate graph without persisting it — the same
/// migrate/validate path `flows_create` and `ProposeWorkflowTool` use — and
/// reports structural errors alongside non-fatal trigger warnings
/// ([`graph_trigger_warnings`]). Backs `openhuman.flows_validate` (PHASE 3c):
/// an authoring surface can call this to preview validity + warnings before a
/// save. Pure (no persistence, no config) — `valid == false` is a normal
/// result, NOT an `Err`; `Err` is reserved for internal serialization faults
/// (there are none on this path today).
pub fn flows_validate(graph_json: Value) -> Outcome<tinyflows_catalog::FlowValidation> {
    use tinyflows_catalog::FlowValidation;
    tracing::debug!(target: "flows", "[flows] flows_validate: validating candidate graph");
    // Split migrate/deserialize (a genuinely single failure) from structural
    // validation (which can surface many problems at once). A pre-validation
    // failure short-circuits with one error; a deserializable graph is then run
    // through `validate_all` so the author sees every structural problem in one
    // pass instead of one round-trip per error.
    let graph = match migrate_and_deserialize_graph(graph_json) {
        Ok(graph) => graph,
        Err(error) => {
            tracing::debug!(target: "flows", %error, "[flows] flows_validate: graph could not be migrated/parsed");
            return Outcome::single_log(
                FlowValidation {
                    valid: false,
                    errors: vec![error.clone()],
                    error_details: vec![tinyflows_catalog::FlowValidationError {
                        code: "unparseable_graph".to_string(),
                        message: error,
                        node_id: None,
                        field: None,
                    }],
                    warnings: Vec::new(),
                },
                "flow validation failed",
            );
        }
    };

    let structural = tinyflows::validate::validate_all(&graph);
    if !structural.is_empty() {
        let error_details: Vec<_> = structural.iter().map(to_flow_validation_error).collect();
        let errors: Vec<String> = error_details.iter().map(|e| e.message.clone()).collect();
        tracing::debug!(
            target: "flows",
            error_count = errors.len(),
            "[flows] flows_validate: graph is structurally invalid"
        );
        return Outcome::single_log(
            FlowValidation {
                valid: false,
                errors,
                error_details,
                warnings: Vec::new(),
            },
            "flow validation failed",
        );
    }

    let error_details = engine_compatibility_errors(&graph);
    if !error_details.is_empty() {
        let errors = error_details
            .iter()
            .map(|error| error.message.clone())
            .collect();
        tracing::debug!(
            target: "flows",
            error_count = error_details.len(),
            "[flows] flows_validate: graph uses an unsupported engine topology"
        );
        return Outcome::single_log(
            FlowValidation {
                valid: false,
                errors,
                error_details,
                warnings: Vec::new(),
            },
            "flow validation failed",
        );
    }

    let warnings = graph_trigger_warnings(&graph);
    for warning in &warnings {
        tracing::warn!(target: "flows", warning = %warning, "[flows] flows_validate: non-fatal validation warning");
    }
    tracing::debug!(
        target: "flows",
        node_count = graph.nodes.len(),
        warning_count = warnings.len(),
        "[flows] flows_validate: graph is structurally valid"
    );
    Outcome::single_log(
        FlowValidation {
            valid: true,
            errors: Vec::new(),
            error_details: Vec::new(),
            warnings,
        },
        "flow validated",
    )
}

/// Imports a workflow definition WITHOUT persisting it (PHASE 4d), normalizing
/// it into a migrated + validated [`WorkflowGraph`] the UI opens as an editable
/// canvas *draft*. Two source formats, selected by `format`:
///
/// - `"native"` — a tinyflows `WorkflowGraph` JSON (the same shape
///   `flows_create` accepts). Run straight through [`validate_and_migrate_graph`].
/// - `"n8n"` — an n8n workflow export, mapped best-effort by
///   [`tinyflows_catalog::import::n8n`] into a `WorkflowGraph` (unmapped
///   node types become annotated placeholders, expressions translated where
///   trivial) and THEN run through the same migrate + validate path, so the
///   host engine is the authority on the result's validity.
/// - `None`/`"auto"` — auto-detect: n8n exports carry a `connections` object /
///   `type`-discriminated nodes ([`n8n_import::looks_like_n8n`]); everything
///   else is treated as native.
///
/// Returns `Err` when the (post-mapping) graph is structurally invalid or the
/// JSON is unparseable — import declines rather than handing the canvas a graph
/// that can't be saved. On success the `warnings` carry every non-fatal import
/// approximation (n8n only; native import is warning-free).
///
/// Like `flows_validate`, this is pure: NO persistence, NO enablement. The
/// user's later Save (the existing `flows_create` gate) is the only write.
pub fn flows_import(
    graph_json: Value,
    format: Option<String>,
) -> Result<Outcome<tinyflows_catalog::FlowImport>, String> {
    use tinyflows_catalog::{import::n8n as n8n_import, FlowImport};

    let requested = format
        .as_deref()
        .unwrap_or("auto")
        .trim()
        .to_ascii_lowercase();
    let is_n8n = match requested.as_str() {
        "n8n" => true,
        "native" | "tinyflows" => false,
        "auto" | "" => n8n_import::looks_like_n8n(&graph_json),
        other => {
            return Err(format!(
                "unknown import format '{other}' (expected 'native' or 'n8n')"
            ))
        }
    };
    tracing::debug!(
        target: "flows",
        requested_format = %requested,
        resolved = if is_n8n { "n8n" } else { "native" },
        "[flows] flows_import: importing workflow definition"
    );

    let (candidate, mut warnings) = if is_n8n {
        let mapped = n8n_import::map_n8n_workflow(&graph_json)?;
        // Re-serialize the mapped graph so it re-enters the exact same
        // migrate + validate path a native import takes (single source of truth
        // for validity), rather than trusting the mapper's in-memory graph.
        let value = serde_json::to_value(&mapped.graph).map_err(|e| e.to_string())?;
        (value, mapped.warnings)
    } else {
        (graph_json, Vec::new())
    };

    let graph = validate_and_migrate_graph(candidate)?;
    // Host-side trigger warnings apply to both formats (e.g. an imported
    // webhook trigger that this host does not yet self-fire).
    warnings.extend(graph_trigger_warnings(&graph));
    tracing::debug!(
        target: "flows",
        node_count = graph.nodes.len(),
        warning_count = warnings.len(),
        "[flows] flows_import: import normalized and validated"
    );
    Ok(Outcome::single_log(
        FlowImport { graph, warnings },
        "flow imported",
    ))
}
