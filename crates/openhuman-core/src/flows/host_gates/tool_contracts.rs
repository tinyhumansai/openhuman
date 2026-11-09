use super::graph_json::nodes;
use crate::config::Config;
use serde_json::Value;

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
// [`crate::integrations::composio::catalog::fetch_live_toolkit_catalog`] — the
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
/// catalog was actually reachable. Once the catalog IS reachable, both
/// checks below are HARD: an unreal slug or a missing required arg rejects
/// the graph outright, unlike the
/// advisory output-field/`split_out.path` WARNs in `graph_wiring_warnings`
/// (Part 2c/2d) — those degrade gracefully because a binding to an unknown
/// field can't be proven wrong, whereas a nonexistent slug or a missing
/// required arg are both provably broken.
/// Enforces the host's curated-only, live-action and argument-shape policy.
pub(crate) async fn validate_tool_contracts(config: &Config, graph: &Value) -> Vec<String> {
    use crate::integrations::composio::contract::validation::{
        validate_tool_contract_nodes, ToolContractNode,
    };

    let nodes: Vec<_> = nodes(graph)
        .filter(|node| node.kind == "tool_call")
        .filter_map(|node| {
            Some(ToolContractNode {
                id: node.id.to_owned(),
                slug: node.config.get("slug")?.as_str()?.to_string(),
                args: node.config.get("args").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    validate_tool_contract_nodes(config, &nodes).await
}
