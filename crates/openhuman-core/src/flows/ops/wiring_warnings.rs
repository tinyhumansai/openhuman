use super::*;

/// Author-time wiring warnings for Composio `tool_call` nodes: flags every
/// **required** arg (per the action's schema, best-effort cached lookup) that
/// is absent or a literal `null` in `config.args` — the exact mis-wiring that
/// would later fail the run's required-arg preflight.
///
/// Static by design: an arg carrying an `=`-expression counts as wired (only
/// the runtime preflight can tell whether it resolves), a `=`-derived slug is
/// skipped (can't know the action), and native `oh:` tools are skipped (no
/// Composio schema). Best-effort like the runtime preflight — no schema, no
/// warning, never a block.
pub(crate) async fn graph_wiring_warnings(config: &Config, graph: &WorkflowGraph) -> Vec<String> {
    use crate::flows::tinyflows::caps::composio_required_args;
    use tinyagents_harness::tool::missing_required_args;

    let mut warnings = Vec::new();
    for node in &graph.nodes {
        if node.kind != tinyflows::model::NodeKind::ToolCall {
            continue;
        }
        let Some(slug) = node.config.get("slug").and_then(Value::as_str) else {
            continue;
        };
        // `=`-derived slugs are resolved at runtime; native tools have no
        // Composio schema to check against.
        if slug.starts_with('=') || slug.starts_with("oh:") {
            continue;
        }
        let Some(required) = composio_required_args(config, slug).await else {
            tracing::debug!(target: "flows", node = %node.id, %slug, "[flows] wiring check: no schema — skipping node");
            continue;
        };
        let args = node.config.get("args").cloned().unwrap_or(Value::Null);
        for missing in missing_required_args(&required, &args) {
            tracing::warn!(
                target: "flows",
                node = %node.id,
                %slug,
                arg = %missing,
                "[flows] wiring check: required arg not wired"
            );
            warnings.push(format!(
                "Node '{}': required arg `{missing}` of `{slug}` is not wired — set \
                 args.{missing}, e.g. \"=nodes.<upstream_id>.item.json.<field>\" (an agent \
                 feeding this value needs an output schema — `output_parser.schema` — so its \
                 fields are addressable).",
                node.id
            ));
        }
    }

    warnings.extend(graph_output_field_warnings(config, graph).await);
    warnings.extend(graph_split_out_path_warnings(config, graph).await);
    warnings
}

/// Author-time WARN (systemic tool-contract fix, Part 2c): any
/// `=nodes.<id>.item.json.data.<field>` binding — anywhere in the graph, not
/// just `tool_call` args — whose `<id>` names a `tool_call` node calling a
/// REAL Composio action with a KNOWN live output schema, but whose `<field>`
/// is not one of that action's real `output_fields`. Also warns (a distinct
/// message) when the binding is missing the `data.` segment entirely — a
/// Composio `tool_call`'s real runtime output always wraps its payload in
/// `data` (`ComposioExecuteResponse`; see
/// [`crate::flows::tinyflows::caps::ToolContract::output_fields`]'s doc),
/// so `=nodes.<id>.item.json.<field>` (no `data.`) is GUARANTEED to resolve
/// `null` even when `<field>` names a real output field — that used to be
/// silently accepted here (B1: the exact bug that produces a hollow run).
/// Advisory, not fatal: a binding to an unknown field could still resolve to
/// something useful at runtime for an action whose output schema is
/// incomplete, so this warns rather than rejects — mirroring
/// `graph_wiring_warnings`'s existing required-arg warnings.
///
/// Skipped entirely when the referenced action's output schema is
/// **unknown** (`ToolContract::output_schema` is `None`) — there is nothing
/// real to check the field against, so warning would just be noise (or a
/// false positive for a still-legitimate binding). Also skipped for a
/// binding that dereferences `.item.<field>` without `.json` on an
/// enveloping node — that shape is already a HARD reject in
/// [`validate_binding_resolvability`], not a warning here.
///
/// Also skipped for a binding that addresses the whole payload
/// (`=nodes.<id>.item.json.data`, e.g. as an agent `input_context`) or one
/// of `ComposioExecuteResponse`'s OTHER top-level envelope fields —
/// `successful`, `error`, `costUsd`, `markdownFormatted` — which live
/// alongside `data`, not inside it. `OpenHumanTools::invoke` serializes the
/// whole `ComposioExecuteResponse` verbatim, so these ARE real
/// `.item.json.<x>` fields with no `data.` prefix; flagging them as
/// "missing the `data.` segment" would rewire an already-correct binding to
/// a nonsense path (e.g. suggesting `.item.json.data.successful`).
pub(super) async fn graph_output_field_warnings(
    config: &Config,
    graph: &WorkflowGraph,
) -> Vec<String> {
    use crate::flows::tinyflows::caps::fetch_live_toolkit_catalog;
    // Reading a graph's `=`-bindings is the engine's grammar, not this host's:
    // both helpers were a private copy here until the gates moved upstream.
    use crate::integrations::composio::contract::toolkit_from_slug;
    use tinyflows::bindings::{collect_expressions, parse_node_binding};

    let mut warnings = Vec::new();
    for node in &graph.nodes {
        for (location, expr) in collect_expressions(&node.config) {
            let Some(binding) = parse_node_binding(&expr) else {
                continue;
            };
            if !binding.through_envelope {
                continue;
            }
            let (ref_id, field_path) = (binding.node_id, binding.field_path);
            let Some(ref_node) = graph.node(&ref_id) else {
                continue;
            };
            if ref_node.kind != NodeKind::ToolCall {
                continue;
            }
            let Some(ref_slug) = ref_node.config.get("slug").and_then(Value::as_str) else {
                continue;
            };
            if ref_slug.starts_with('=') || ref_slug.starts_with("oh:") {
                continue;
            }
            let Some(ref_toolkit) = toolkit_from_slug(ref_slug) else {
                continue;
            };
            let Some(catalog) = fetch_live_toolkit_catalog(config, &ref_toolkit).await else {
                continue;
            };
            let Some(contract) = catalog
                .iter()
                .find(|c| c.slug.eq_ignore_ascii_case(ref_slug))
            else {
                continue;
            };
            // B12: a real-output probe (`get_tool_output_sample`) for this
            // exact slug overrides the schema-derived `output_fields` — most
            // relevant for an action whose live listing publishes no output
            // schema at all (e.g. every GitHub action, verified live).
            let contract = crate::flows::tinyflows::caps::apply_probe_override(contract.clone());
            // Nothing real to check `field_path` against — schema unknown AND
            // no probed output fields either.
            if contract.output_schema.is_none() && contract.output_fields.is_empty() {
                continue;
            }

            // Whole-payload access (`.item.json.data`, e.g. an agent's
            // `input_context`) or one of `ComposioExecuteResponse`'s OTHER
            // top-level envelope fields — these live alongside `data`, not
            // inside it, and are real fields regardless of this action's
            // `output_fields` (see this fn's doc). Not a "missing `data.`"
            // mistake.
            const COMPOSIO_ENVELOPE_METADATA_FIELDS: &[&str] =
                &["successful", "error", "costUsd", "markdownFormatted"];
            if field_path == "data"
                || COMPOSIO_ENVELOPE_METADATA_FIELDS
                    .contains(&field_path.split('.').next().unwrap_or(&field_path))
            {
                continue;
            }

            // A real Composio tool_call's payload is always nested one level
            // under `data` (see this fn's doc) — a binding missing that
            // segment is wrong regardless of whether the rest of the path
            // happens to name a real field.
            let Some(field) = field_path.strip_prefix("data.") else {
                tracing::warn!(
                    target: "flows",
                    node = %node.id,
                    %location,
                    ref_node = %ref_id,
                    ref_slug,
                    %field_path,
                    "[flows] wiring check: downstream binding is missing the Composio `data.` wrapper segment"
                );
                warnings.push(format!(
                    "Node '{}': binding `{location}` (`{expr}`) reads `.item.json.{field_path}` off \
                     tool_call `{ref_id}` (`{ref_slug}`), but a Composio tool_call's real output \
                     wraps its payload in `data` — this resolves null at runtime. Bind via \
                     `=nodes.{ref_id}.item.json.data.{field_path}` instead.",
                    node.id
                ));
                continue;
            };
            let field = field.split('.').next().unwrap_or(field);
            if !contract.output_fields.iter().any(|f| f == field) {
                tracing::warn!(
                    target: "flows",
                    node = %node.id,
                    %location,
                    ref_node = %ref_id,
                    ref_slug,
                    %field,
                    output_fields = ?contract.output_fields,
                    "[flows] wiring check: downstream binding reads a field not in the tool's real output_fields"
                );
                warnings.push(format!(
                    "Node '{}': binding `{location}` (`{expr}`) reads field `{field}` off \
                     tool_call `{ref_id}` (`{ref_slug}`), but that is not one of its real \
                     output fields ({}) — call get_tool_contract {{ slug: \"{ref_slug}\" }} to \
                     see the real output field names.",
                    node.id,
                    contract.output_fields.join(", "),
                ));
            }
        }
    }
    warnings
}

/// Given a Composio action's payload-only `output_schema` (see
/// [`crate::flows::tinyflows::caps::ToolContract::output_fields`]'s doc —
/// NEVER includes the runtime `data` envelope) and a `split_out.path`
/// addressed relative to the ENVELOPE (`json.<envelope_field…>`, e.g.
/// `"json.data"` or `"json.data.issues"`), resolves whether the path lands on
/// something that is DEFINITELY not an array.
///
/// `Some(true)` — non-array (an object or scalar): a `split_out` over this
/// path fans out over exactly ONE item, the classic "wrong array path"
/// signal [`graph_split_out_path_warnings`]'s generic enforcement flags.
/// `Some(false)` — array: the path is fine. `None` — the path can't be
/// resolved against the schema at all (an unpublished/unknown nested field,
/// or a path missing the `data.` segment entirely) — stay silent rather than
/// guess; that's a distinct failure mode from "resolves to a non-array".
fn schema_says_path_is_non_array(output_schema: &Value, configured_path: &str) -> Option<bool> {
    let relative = configured_path
        .strip_prefix("json.")
        .unwrap_or(configured_path);
    if relative == "data" {
        // Whole-payload access (`json.data`) — non-array unless the payload's
        // own root schema type is literally "array" (a bare-array response,
        // e.g. a REST endpoint that returns `[...]` directly), in which case
        // `json.data` legitimately IS the real list.
        let ty = output_schema.get("type").and_then(Value::as_str)?;
        return Some(ty != "array");
    }
    let rest = relative.strip_prefix("data.").filter(|r| !r.is_empty())?;
    let mut node = output_schema;
    for seg in rest.split('.') {
        node = node.get("properties")?.get(seg)?;
    }
    let ty = node.get("type").and_then(Value::as_str)?;
    Some(ty != "array")
}

/// Author-time WARN/suggest (systemic tool-contract fix, Part 2d, extended by
/// B12): a `split_out` node whose direct predecessor is a `tool_call` calling
/// a REAL Composio action, checked two ways:
///
/// 1. **KNOWN `primary_array_path`** (see
///    [`crate::flows::tinyflows::caps::compute_composio_array_path`] —
///    this already bakes in the `data.` segment Composio's execute-response
///    wrapper adds, so `expected` below comes out `"json.data.<…>"` with no
///    extra handling needed here — and, via
///    [`crate::flows::tinyflows::caps::apply_probe_override`], a real
///    `get_tool_output_sample` probe for this slug overrides a schema that
///    never named an array at all): if the configured `config.path` doesn't match the
///    `json.<primary_array_path>` convention, suggest the real path.
/// 2. **UNKNOWN `primary_array_path`, but a KNOWN `output_schema`/probe that
///    proves the configured path is definitely NOT an array** (B12
///    enforcement, "regardless" of whether a correct path can be suggested —
///    catches the class at build time even when nothing to suggest is
///    derivable): warn generically. This is exactly the live bug this fix
///    closes — `GITHUB_LIST_REPOSITORY_ISSUES` publishes no output schema at
///    all, so a builder without a probe guessed the whole-payload
///    `"json.data"`, silently fanning out over ONE item (the `{issues:
///    [...]}` container) instead of the real per-issue list.
///
/// Both are advisory: a mismatched/non-array path degrades the fan-out (or
/// silently produces one item instead of many) rather than crashing.
///
/// Skipped entirely when `split_out`'s predecessor isn't a `tool_call` at all
/// (no envelope/array-path convention applies), or when NEITHER a
/// `primary_array_path` NOR an `output_schema` is known (truly nothing to
/// check against).
async fn graph_split_out_path_warnings(config: &Config, graph: &WorkflowGraph) -> Vec<String> {
    use crate::flows::tinyflows::caps::{apply_probe_override, fetch_live_toolkit_catalog};
    use crate::integrations::composio::contract::toolkit_from_slug;

    let mut warnings = Vec::new();
    for node in &graph.nodes {
        if node.kind != NodeKind::SplitOut {
            continue;
        }
        let configured_path = node.config.get("path").and_then(Value::as_str);

        for edge in graph.edges.iter().filter(|e| e.to_node == node.id) {
            let Some(pred) = graph.node(&edge.from_node) else {
                continue;
            };
            if pred.kind != NodeKind::ToolCall {
                continue;
            }
            let Some(pred_slug) = pred.config.get("slug").and_then(Value::as_str) else {
                continue;
            };
            if pred_slug.starts_with('=') || pred_slug.starts_with("oh:") {
                continue;
            }
            let Some(pred_toolkit) = toolkit_from_slug(pred_slug) else {
                continue;
            };
            let Some(catalog) = fetch_live_toolkit_catalog(config, &pred_toolkit).await else {
                continue;
            };
            let Some(contract) = catalog
                .iter()
                .find(|c| c.slug.eq_ignore_ascii_case(pred_slug))
            else {
                continue;
            };
            // B12: a real-output probe overrides the schema-derived
            // `primary_array_path` for this exact slug when one is cached.
            let contract = apply_probe_override(contract.clone());

            match contract.primary_array_path.as_deref() {
                Some(primary) => {
                    let expected = format!("json.{primary}");
                    if configured_path != Some(expected.as_str()) {
                        tracing::warn!(
                            target: "flows",
                            node = %node.id,
                            predecessor = %pred.id,
                            pred_slug,
                            configured_path,
                            %expected,
                            "[flows] wiring check: split_out.path does not match the predecessor tool's real array path"
                        );
                        let configured_display = configured_path
                            .map(|p| format!("\"{p}\""))
                            .unwrap_or_else(|| "unset".to_string());
                        warnings.push(format!(
                            "Node '{}': split_out.path is {configured_display} but its predecessor \
                             tool_call `{}` (`{pred_slug}`) wraps its real array at `{expected}` — set \
                             config.path to \"{expected}\" to fan out over the actual response list.",
                            node.id, pred.id,
                        ));
                    }
                }
                // No known array anywhere in this action's real output — the
                // generic non-array enforcement is the only thing left that
                // can catch a wrong path here (nothing to suggest, but a
                // known-non-array hit is still a strong signal).
                None => {
                    let Some(cp) = configured_path else { continue };
                    let Some(schema) = contract.output_schema.as_ref() else {
                        continue;
                    };
                    if schema_says_path_is_non_array(schema, cp) == Some(true) {
                        tracing::warn!(
                            target: "flows",
                            node = %node.id,
                            predecessor = %pred.id,
                            pred_slug,
                            configured_path = cp,
                            "[flows] wiring check: split_out.path resolves to a non-array — likely the wrong array path"
                        );
                        warnings.push(format!(
                            "Node '{}': split_out.path is \"{cp}\" but tool_call `{}` (`{pred_slug}`)'s \
                             known real output does not name an array at that path (or names no array \
                             property at all) — this fans out over a single object instead of a real \
                             list. If the action's real output nests the list under a named field (e.g. \
                             `data.issues`), call get_tool_output_sample {{ slug: \"{pred_slug}\" }} to \
                             sample the real response, then re-check with get_tool_contract.",
                            node.id, pred.id,
                        ));
                    }
                }
            }
        }
    }
    warnings
}
