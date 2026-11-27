//! Controller schemas and dispatch table for the search controllers in the
//! `tools` RPC namespace.
//!
//! The Tauri host drives Parallel-backed web search itself during onboarding
//! (orchestration in the renderer; external calls still go through the core's
//! auth / proxy layer), and MCP clients call the provider-pinned SearXNG
//! search. The wire names stay `tools.*`; the handlers live in [`super::rpc`].

use crate::core::all::RegisteredController;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::rpc::{
    handle_searxng_search, handle_web_answer, handle_web_contents, handle_web_search,
};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("tools_web_search"),
        schemas("tools_web_answer"),
        schemas("tools_web_contents"),
        schemas("tools_searxng_search"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("tools_web_search"),
            handler: handle_web_search,
        },
        RegisteredController {
            schema: schemas("tools_web_answer"),
            handler: handle_web_answer,
        },
        RegisteredController {
            schema: schemas("tools_web_contents"),
            handler: handle_web_contents,
        },
        RegisteredController {
            schema: schemas("tools_searxng_search"),
            handler: handle_searxng_search,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "tools_web_search" => ControllerSchema {
            namespace: "tools",
            function: "web_search",
            description: "Ranked web search through the configured search providers \
                          (the `search` role: first usable provider, then fallbacks). \
                          Returns structured results and the provider that answered.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Search query string.",
                    required: true,
                },
                FieldSchema {
                    name: "max_results",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max results (1-20, default from search settings).",
                    required: false,
                },
                FieldSchema {
                    name: "provider",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Pin one provider (exa, brave, tavily, querit, seltz, searxng, tinyfish); disables fallback.",
                    required: false,
                },
            ],
            outputs: search_outputs("Each item: {title, url, snippet?, published?}."),
        },
        "tools_web_answer" => ControllerSchema {
            namespace: "tools",
            function: "web_answer",
            description: "Grounded answer with citations (the `answer` role: Gemini with \
                          Google Search grounding by default). `depth: deep` runs deep \
                          research when a Gemini key is configured.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "The question to answer.",
                    required: true,
                },
                FieldSchema {
                    name: "depth",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Enum {
                        variants: vec!["quick", "deep"],
                    })),
                    comment: "quick (default) or deep.",
                    required: false,
                },
                FieldSchema {
                    name: "provider",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Pin one provider (gemini, exa); disables fallback.",
                    required: false,
                },
            ],
            outputs: search_outputs("Sources the answer drew on, when the provider lists them."),
        },
        "tools_web_contents" => ControllerSchema {
            namespace: "tools",
            function: "web_contents",
            description: "Fetch and extract page contents for known URLs (the `contents` role).",
            inputs: vec![
                FieldSchema {
                    name: "urls",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "One or more page URLs.",
                    required: true,
                },
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional focus for extraction.",
                    required: false,
                },
                FieldSchema {
                    name: "provider",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Pin one provider (exa, tavily, tinyfish); disables fallback.",
                    required: false,
                },
            ],
            outputs: search_outputs("Each item: {title, url, snippet?} with the extracted text."),
        },
        "tools_searxng_search" => ControllerSchema {
            namespace: "tools",
            function: "searxng_search",
            description: "Web search pinned to the user's self-hosted SearXNG instance. \
                          Requires SearXNG to be enabled in search settings.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Search query string.",
                    required: true,
                },
                FieldSchema {
                    name: "max_results",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max results (1-20).",
                    required: false,
                },
            ],
            outputs: search_outputs("Each item: {title, url, snippet?, published?}."),
        },
        _ => ControllerSchema {
            namespace: "tools",
            function: "unknown",
            description: "Unknown tools controller.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

/// Shared output shape of the search controllers.
fn search_outputs(results_comment: &'static str) -> Vec<FieldSchema> {
    vec![
        FieldSchema {
            name: "results",
            ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
            comment: results_comment,
            required: true,
        },
        FieldSchema {
            name: "provider",
            ty: TypeSchema::String,
            comment: "Display name of the provider that answered.",
            required: true,
        },
        FieldSchema {
            name: "answer",
            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
            comment: "Synthesized answer, for answer-capable providers.",
            required: false,
        },
        FieldSchema {
            name: "citations",
            ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
            comment: "Each item: {url, title?}.",
            required: true,
        },
        FieldSchema {
            name: "fallback_from",
            ty: TypeSchema::Array(Box::new(TypeSchema::String)),
            comment: "Providers tried and skipped before the one that answered.",
            required: true,
        },
    ]
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
