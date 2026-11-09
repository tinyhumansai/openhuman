//! Schemas for the authoring copilot (`build`/`build_cancel`), the tool
//! catalog it browses, and workflow discovery suggestions.

use super::{
    id_input, stream_request_id_input, stream_thread_id_input, suggestions_output,
    ControllerSchema, FieldSchema, TypeSchema,
};

/// Builds the schema for `function` when this group owns it.
pub(super) fn lookup(function: &str) -> Option<ControllerSchema> {
    match function {
        "build" => Some(ControllerSchema {
            namespace: "flows",
            function: "build",
            description: "Run the workflow_builder agent for one authoring turn. `mode` selects \
                          create (first draft from `instruction`), revise (refine the injected \
                          `graph`), repair (diagnose a failed `run_id` and fix), or build \
                          (instant-create: build + dry-run + propose against `flow_id`; \
                          propose-only, see #4596). The server renders the agent's brief — the \
                          frontend no longer crafts prompts. Returns `{ proposal, assistant_text, \
                          error }`, where `proposal` is the `{ type: 'workflow_proposal', name, \
                          graph, require_approval, summary, warnings }` the agent produced (or \
                          null). No mode auto-persists a graph; save/enable/run stay behind the \
                          user's explicit action.",
            inputs: vec![
                FieldSchema {
                    name: "mode",
                    ty: TypeSchema::String,
                    comment: "One of: `create` | `revise` | `repair` | `build`.",
                    required: true,
                },
                FieldSchema {
                    name: "instruction",
                    ty: TypeSchema::String,
                    comment: "The user's ask: description (create/build) or change instruction \
                              (revise); optional note for repair.",
                    required: false,
                },
                FieldSchema {
                    name: "graph",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "The current draft WorkflowGraph, injected as context for \
                              revise/repair/build.",
                    required: false,
                },
                FieldSchema {
                    name: "flow_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Saved flow id — required for `build` (save target); optional \
                              elsewhere (lets the agent run_flow it to test, with confirmation).",
                    required: false,
                },
                FieldSchema {
                    name: "run_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Failed run id (== thread id) for `repair`, so the agent can \
                              get_flow_run it.",
                    required: false,
                },
                FieldSchema {
                    name: "error",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Run-level error message for `repair`, if known.",
                    required: false,
                },
                FieldSchema {
                    name: "failing_node_ids",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Node ids implicated in the failure, for `repair` (array of strings).",
                    required: false,
                },
                stream_thread_id_input(),
                stream_request_id_input(),
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "`{ proposal, assistant_text, error }` — `proposal` is the workflow \
                          proposal the agent produced (or null); `error` is set if the run failed \
                          but a prior proposal was still captured.",
                required: true,
            }],
        }),
        "build_cancel" => Some(ControllerSchema {
            namespace: "flows",
            function: "build_cancel",
            description: "Cancel the in-flight `flows_build` (Workflow Copilot) turn streaming \
                          into `thread_id` — the real cancellation behind the composer's Stop \
                          button. When `request_id` is given, the cancel only fires if it \
                          matches the turn currently registered on the thread (a stale Stop for \
                          a superseded request can't kill a newer turn); omit it to cancel \
                          whatever turn is on the thread. `cancelled: false` is not an error — it \
                          just means nothing was in flight (already settled, or never started).",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "The copilot's dedicated chat thread id (the same `thread_id` \
                              passed to `flows.build`'s streaming params).",
                    required: true,
                },
                FieldSchema {
                    name: "request_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Per-turn correlation id to scope the cancel to (matches the \
                              `request_id` `flows.build` streamed with). Omit to cancel \
                              unscoped.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![FieldSchema {
                        name: "cancelled",
                        ty: TypeSchema::Bool,
                        comment: "True when an in-flight build turn was found and signalled to \
                                  cancel.",
                        required: true,
                    }],
                },
                comment: "Cancellation result payload.",
                required: true,
            }],
        }),
        "search_tool_catalog" => Some(ControllerSchema {
            namespace: "flows",
            function: "search_tool_catalog",
            description: "Search the live Composio tool catalog (secret-free) for the in-canvas \
                          tool browser — the same core as the agent's search_tool_catalog tool.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Keyword query matched against slug / toolkit / description.",
                    required: true,
                },
                FieldSchema {
                    name: "toolkit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Restrict to one toolkit slug (e.g. `gmail`); omit to search all.",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max results (default 25).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "tools",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Matches: { slug, toolkit, description, required_args, output_fields, primary_array_path, featured }.",
                required: true,
            }],
        }),
        "get_tool_contract" => Some(ControllerSchema {
            namespace: "flows",
            function: "get_tool_contract",
            description: "Fetch one Composio action's full contract (secret-free) for the canvas \
                          tool browser — the same core as the agent's get_tool_contract tool.",
            inputs: vec![FieldSchema {
                name: "slug",
                ty: TypeSchema::String,
                comment: "The exact Composio action slug (e.g. `GMAIL_SEND_EMAIL`).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "contract",
                ty: TypeSchema::Json,
                comment: "The action contract: { slug, toolkit, description, required_args, input_schema, output_fields, output_schema, primary_array_path, is_curated }.",
                required: true,
            }, FieldSchema {
                name: "runtime_gate",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Present when this real action is outside the toolkit's curated runtime allowlist and will be rejected on every real run.",
                required: false,
            }],
        }),
        "discover" => Some(ControllerSchema {
            namespace: "flows",
            function: "discover",
            description: "Run the read-only Flow Scout: it reads the user's \
                          memory/threads/people/connections/existing flows and records a handful \
                          of concrete, buildable workflow suggestions for the Flows page. It never \
                          creates, enables, or runs a flow — turning a suggestion into a real flow \
                          is the user's separate 'Build this' action. Returns the active (new) \
                          suggestions after the run.",
            inputs: vec![stream_thread_id_input(), stream_request_id_input()],
            outputs: vec![suggestions_output()],
        }),
        "list_suggestions" => Some(ControllerSchema {
            namespace: "flows",
            function: "list_suggestions",
            description: "List persisted workflow suggestions. Filter by lifecycle `status` \
                          (`new` | `dismissed` | `built`); omit to return every status.",
            inputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Lifecycle filter: `new` (active cards) | `dismissed` | `built`. \
                          Omit for all.",
                required: false,
            }],
            outputs: vec![suggestions_output()],
        }),
        "dismiss_suggestion" => Some(ControllerSchema {
            namespace: "flows",
            function: "dismiss_suggestion",
            description: "Dismiss a workflow suggestion (the user rejected the card). The row is \
                          kept so a later discovery run dedupes against it and won't re-surface \
                          the idea.",
            inputs: vec![id_input("Identifier of the suggestion to dismiss.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "`{ id, dismissed }` — `dismissed` is false if the id was unknown.",
                required: true,
            }],
        }),
        "mark_suggestion_built" => Some(ControllerSchema {
            namespace: "flows",
            function: "mark_suggestion_built",
            description: "Mark a suggestion as built — called after the user saves a flow authored \
                          from it, so it drops out of the active cards.",
            inputs: vec![id_input("Identifier of the suggestion that was built.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "`{ id, built }` — `built` is false if the id was unknown.",
                required: true,
            }],
        }),
        _ => None,
    }
}
