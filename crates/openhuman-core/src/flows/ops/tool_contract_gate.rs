use super::*;

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
// [`crate::flows::tinyflows::caps::fetch_live_toolkit_catalog`] — the
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
    use crate::integrations::composio::providers::catalog_for_toolkit;
    catalog_for_toolkit(toolkit).is_some()
}

pub(crate) async fn validate_tool_contracts(config: &Config, graph: &WorkflowGraph) -> Vec<String> {
    use crate::flows::tinyflows::caps::fetch_live_toolkit_catalog;
    use crate::integrations::composio::contract::toolkit_from_slug;
    use tinyagents_harness::tool::{missing_required_args, unsupported_arg_names};

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
