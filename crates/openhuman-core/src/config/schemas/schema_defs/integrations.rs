//! Schemas for third-party integration settings: web search and Composio triggers.

use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::super::helpers::{json_output, optional_bool, optional_string};

pub(super) fn lookup(function: &str) -> Option<ControllerSchema> {
    match function {
"update_search_settings" => Some( ControllerSchema {
            namespace: "config",
            function: "update_search_settings",
            description: "Update search providers, presentation, routes, and private API credentials.",
            inputs: vec![
                optional_bool("enabled", "Whether TinySearch is enabled globally."),
                FieldSchema { name: "enabled_providers", ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::String)))), comment: "Selected search providers; empty list disables all providers. Managed and direct Parallel share one route; explicit Parallel takes precedence.", required: false },
                optional_string("presentation", "all_tools | router | one_provider."),
                optional_string("presentation_provider", "Provider selected for one_provider or router default."),
                optional_string("parallel_route", "direct | backend."),
                optional_string("gemini_route", "direct | backend."),
                optional_string("gemini_api_key", "Gemini direct API key (empty string clears)."),
                optional_string(
                    "engine",
                    "Legacy engine selector; disabled still disables search.",
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
                optional_string(
                    "parallel_api_key",
                    "Parallel API key (empty string clears the stored key).",
                ),
                optional_string(
                    "brave_api_key",
                    "Brave Search API key (empty string clears the stored key).",
                ),
                optional_string(
                    "querit_api_key",
                    "Querit API key (empty string clears the stored key).",
                ),
                optional_string(
                    "exa_api_key",
                    "Exa API key (empty string clears the stored key).",
                ),
                optional_string(
                    "tavily_api_key",
                    "Tavily API key (empty string clears the stored key).",
                ),
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
            outputs: vec![json_output("snapshot", "Updated config snapshot.")],
        }),
"get_search_settings" => Some( ControllerSchema {
            namespace: "config",
            function: "get_search_settings",
            description:
                "Read search engine settings. API keys are surfaced as presence booleans only.",
            inputs: vec![],
            outputs: vec![json_output(
                "settings",
                "Engine, effective engine, limits, and per-provider configuration flags.",
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
