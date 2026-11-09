//! Schemas for third-party integration settings: web search and Composio triggers.

use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::super::helpers::{json_output, optional_bool, optional_string};

pub(super) fn lookup(function: &str) -> Option<ControllerSchema> {
    match function {
        "update_search_settings" => Some(ControllerSchema {
            namespace: "config",
            function: "update_search_settings",
            description: "Update search providers, their routes (managed or own key), per-role provider order, limits, and the web-access allowlist. Keys are write-only.",
            inputs: vec![
                optional_bool("enabled", "Whether web search is enabled globally."),
                optional_string(
                    "engine",
                    "Legacy single-engine selector from older clients: disabled | managed | brave | querit | exa | tavily.",
                ),
                FieldSchema {
                    name: "providers",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Per-provider changes keyed by provider id (exa, gemini, tinyfish, brave, tavily, querit, seltz, searxng): {enabled?, route?: managed|direct, api_key? (empty clears), base_url? (searxng)}.",
                    required: false,
                },
                FieldSchema {
                    name: "roles",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Ordered provider list per role (search | answer | contents). The first usable provider serves the role; an empty list restores the default order.",
                    required: false,
                },
                optional_string("presentation", "roles | all_tools | router | one_provider."),
                optional_string(
                    "presentation_provider",
                    "Provider for one_provider or the router default; empty clears.",
                ),
                FieldSchema {
                    name: "max_results",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum results per query (1-20).",
                    required: false,
                },
                FieldSchema {
                    name: "timeout_secs",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Per-request timeout in seconds (1-120).",
                    required: false,
                },
                FieldSchema {
                    name: "allowed_domains",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Websites the assistant may open/read (web_fetch/curl). Exact hosts match their subdomains; \"*\" allows all public sites; empty blocks all web access.",
                    required: false,
                },
                FieldSchema {
                    name: "allow_all",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "\"Allow all sites\" toggle. true sets the allowlist to [\"*\"]; false drops the wildcard, keeping explicit hosts.",
                    required: false,
                },
            ],
            outputs: vec![json_output(
                "settings",
                "Updated search settings (same shape as get_search_settings).",
            )],
        }),
        "get_search_settings" => Some(ControllerSchema {
            namespace: "config",
            function: "get_search_settings",
            description: "Read search settings: each provider's route, key presence and status, the per-role provider order, and which providers serve each role now. Keys are never returned.",
            inputs: vec![],
            outputs: vec![json_output(
                "settings",
                "enabled, presentation, limits, managed_available, providers[], roles, effective_roles, allowed_domains, allow_all.",
            )],
        }),
        "update_composio_trigger_settings" => Some(ControllerSchema {
            namespace: "config",
            function: "update_composio_trigger_settings",
            description: "Update Composio trigger-triage settings. When triage is disabled the \
                 local LLM is NOT invoked per trigger — events are still archived to \
                 trigger history.",
            inputs: vec![
                optional_bool(
                    "triage_disabled",
                    "When true, skip the LLM triage turn for all Composio triggers globally.",
                ),
                FieldSchema {
                    name: "triage_disabled_toolkits",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Toolkit slugs that skip LLM triage (e.g. [\"gmail\", \"slack\"]).",
                    required: false,
                },
            ],
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        }),
        "get_composio_trigger_settings" => Some(ControllerSchema {
            namespace: "config",
            function: "get_composio_trigger_settings",
            description: "Read current Composio trigger-triage settings.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "triage_disabled",
                    ty: TypeSchema::Bool,
                    comment: "Whether the global triage-disabled flag is set.",
                    required: true,
                },
                FieldSchema {
                    name: "triage_disabled_toolkits",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Toolkit slugs that skip LLM triage.",
                    required: true,
                },
            ],
        }),
        _ => None,
    }
}
