//! Host overlay on the tinyflows node-kind catalog (P1.2 / audit finding F2).
//!
//! The portable, model-level contracts live in the tinyflows crate
//! ([`tinyflows::catalog`]) — config fields, ports, examples, and the
//! structural gotchas that are true of the DSL everywhere. This module is the
//! **thin host layer**: it takes those contracts and appends the facts only
//! *this* host knows — what a `tool_call` slug resolves to (a Composio action
//! slug or an `oh:` native tool), that a Composio result is wrapped in `data`,
//! how an `agent` node receives its data (`input_context`), and which trigger
//! kinds actually dispatch here. Keeping the vendor-specific knowledge here
//! preserves tinyflows' host-agnostic invariant.
//!
//! The `list_node_kinds` / `get_node_kind_contract` agent tools and the
//! `propose_workflow` description all read the *overlaid* view via
//! [`all_node_kind_contracts`] / [`node_kind_contract`].

pub use tinyflows::catalog::{ConfigField, NodeKindContract, PortSpec, NODE_KINDS};

/// Appends this host's vendor-specific caveats to a portable tinyflows
/// contract. One arm per kind that has host-owned facts; the rest pass through
/// unchanged.
fn apply_host_overlay(contract: NodeKindContract) -> NodeKindContract {
    match contract.kind.as_str() {
        "trigger" => contract
            .with_note(
                "In THIS host only manual / schedule / app_event actually dispatch today; the \
                 other kinds save but never self-run (flows_validate warns).",
            )
            .with_note(
                "trigger_kind=app_event also needs config.toolkit + config.trigger_slug (the \
                 Composio app + event to match).",
            ),
        "agent" => contract
            .with_note(
                "Data reaches the agent via config.input_context — an explicit =-binding: \
                 \"=item\" (direct predecessor), \"=items\" (all inputs, for a fan-in), or \
                 \"=nodes.<id>.item.json\" (a specific upstream node). The agent has NO automatic \
                 access to the upstream item.",
            )
            .with_note(
                "config.prompt must be PLAIN natural-language text — no leading = and no .item \
                 woven into the prose. A prompt written as a =expression built from prose silently \
                 resolves to null and hands the agent an EMPTY prompt (rejected by the \
                 binding-resolvability gate).",
            ),
        "tool_call" => contract
            .with_note(
                "config.slug is a real Composio action slug (from search_tool_catalog, e.g. \
                 GMAIL_SEND_EMAIL) OR oh:<tool_name> for a native OpenHuman tool (e.g. \
                 oh:web_search). A hallucinated/typo'd slug is a hard reject.",
            )
            .with_note(
                "Before wiring, call get_tool_contract { slug }: wire EVERY required_arg into \
                 config.args using the input_schema's REAL property names (a guessed key is \
                 rejected). Composio actions also need config.connection_ref for the account; oh: \
                 tools do not.",
            )
            .with_note(
                "A Composio tool_call's output is wrapped in `data` (ComposioExecuteResponse) — \
                 bind downstream as =nodes.<id>.item.json.data.<field>, NOT .item.json.<field>. To \
                 split_out over its result list, use get_tool_contract's primary_array_path \
                 prefixed with `json.` (e.g. \"json.data.messages\").",
            ),
        "http_request" => contract.with_note(
            "config.connection_ref is an http_cred:<name> credential for authentication.",
        ),
        "code" => contract.with_note(
            "A code node's output is NOT `data`-wrapped (unlike a Composio tool_call) — bind \
             downstream as =nodes.<id>.item.json.<field>.",
        ),
        "split_out" => contract.with_note(
            "For a Composio source whose get_tool_contract primary_array_path is null, do NOT \
             default the path to \"json.data\" (that targets the whole payload container and \
             yields one item) — probe the real array path with get_tool_output_sample instead.",
        ),
        _ => contract,
    }
}

/// All 12 node-kind contracts with this host's overlay applied, in
/// [`NODE_KINDS`] order.
pub fn all_node_kind_contracts() -> Vec<NodeKindContract> {
    tinyflows::catalog::all_contracts()
        .into_iter()
        .map(apply_host_overlay)
        .collect()
}

/// The overlaid contract for one node kind, or `None` if `kind` is not one of
/// the 12.
pub fn node_kind_contract(kind: &str) -> Option<NodeKindContract> {
    tinyflows::catalog::contract_for(kind).map(apply_host_overlay)
}

/// Renders the compact, one-line-per-kind node-kind enumeration used to keep
/// `propose_workflow`'s description honest against the typed contracts (drift
/// test). Format: `kind [required config.a/config.b; optional config.c] —
/// summary`, joined by ` | `.
pub fn render_node_kinds_line() -> String {
    all_node_kind_contracts()
        .iter()
        .map(|c| {
            let required: Vec<&str> = c
                .config_fields
                .iter()
                .filter(|f| f.required)
                .map(|f| f.name.as_str())
                .collect();
            let optional: Vec<&str> = c
                .config_fields
                .iter()
                .filter(|f| !f.required)
                .map(|f| f.name.as_str())
                .collect();
            let mut cfg = String::new();
            if !required.is_empty() {
                cfg.push_str(&format!("required config.{}", required.join("/config.")));
            }
            if !optional.is_empty() {
                if !cfg.is_empty() {
                    cfg.push_str("; ");
                }
                cfg.push_str(&format!("optional config.{}", optional.join("/config.")));
            }
            if cfg.is_empty() {
                format!("{} ({})", c.kind, c.summary)
            } else {
                format!("{} [{}] — {}", c.kind, cfg, c.summary)
            }
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_preserves_all_12_kinds() {
        assert_eq!(all_node_kind_contracts().len(), 12);
        for kind in NODE_KINDS {
            assert!(node_kind_contract(kind).is_some(), "missing {kind}");
        }
        assert!(node_kind_contract("not_a_kind").is_none());
    }

    #[test]
    fn tool_call_overlay_adds_host_composio_facts() {
        let c = node_kind_contract("tool_call").unwrap();
        let notes = c.notes.join("\n");
        // Host facts that must NOT live in the portable crate.
        assert!(notes.contains("Composio"), "{notes}");
        assert!(notes.contains("oh:"), "{notes}");
        assert!(notes.contains("data"), "{notes}");
        assert!(notes.contains("get_tool_contract"), "{notes}");
    }

    #[test]
    fn agent_overlay_adds_input_context_guidance() {
        let c = node_kind_contract("agent").unwrap();
        assert!(c.notes.iter().any(|n| n.contains("input_context")));
    }

    #[test]
    fn trigger_overlay_names_the_host_dispatch_set() {
        let c = node_kind_contract("trigger").unwrap();
        assert!(c.notes.iter().any(|n| n.contains("app_event")));
    }

    #[test]
    fn merge_has_no_overlay_and_stays_portable() {
        // A kind with no host facts is byte-identical to the portable contract.
        assert_eq!(
            node_kind_contract("merge").unwrap(),
            tinyflows::catalog::contract_for("merge").unwrap()
        );
    }

    #[test]
    fn rendered_line_covers_every_kind_and_required_field() {
        let line = render_node_kinds_line();
        for c in all_node_kind_contracts() {
            assert!(
                line.contains(&c.kind),
                "rendered line missing kind {}",
                c.kind
            );
            for f in c.config_fields.iter().filter(|f| f.required) {
                assert!(
                    line.contains(&format!("config.{}", f.name)),
                    "rendered line missing required field config.{} for {}",
                    f.name,
                    c.kind
                );
            }
        }
    }
}
