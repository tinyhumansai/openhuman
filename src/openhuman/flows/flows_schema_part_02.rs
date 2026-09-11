use super::*;

pub(super) fn lookup(function: &str) -> Option<ControllerSchema> {
    match function {
"mark_suggestion_built" => Some( ControllerSchema {
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
"approval_manifest" => Some( ControllerSchema {
            namespace: "flows",
            function: "approval_manifest",
            description:
                "Compute the approval manifest for a saved flow (by id) or a candidate graph: \
                 every ApprovalGate permission a run will prompt for, joined against the flow's \
                 existing flow_tool_trust grants — the data behind the consolidated save+enable \
                 pre-authorization card. Entries carry kind approvable|blocked|dynamic|agent.",
            inputs: vec![
                FieldSchema {
                    name: "id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Saved flow id. Provide this or 'graph'.",
                    required: false,
                },
                FieldSchema {
                    name: "graph",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Candidate WorkflowGraph to inspect (no trust join without an id).",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "entries",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                    comment:
                        "One per relevant node/tool: {kind: approvable|blocked|dynamic|agent, \
                         node_id, tool_name?, label, class?}.",
                    required: true,
                },
                FieldSchema {
                    name: "missing",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Approvable trust keys the flow does not yet hold.",
                    required: true,
                },
                FieldSchema {
                    name: "already_trusted",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Approvable trust keys already granted to this flow. Empty when \
                              gate_installed is false — no gate means no grant was ever made.",
                    required: true,
                },
                FieldSchema {
                    name: "gate_installed",
                    ty: TypeSchema::Bool,
                    comment:
                        "False when the approval gate is disabled — nothing ever prompts, so \
                         missing is empty by definition.",
                    required: true,
                },
            ],
        }),
"required_connections" => Some( ControllerSchema {
            namespace: "flows",
            function: "required_connections",
            description: "Compute which Composio toolkits a candidate graph needs and whether each \
                          is connected — the data behind the canvas/proposal \"Connect <toolkit>\" \
                          CTAs. Native oh: tools and http_request nodes need no connection.",
            inputs: vec![FieldSchema {
                name: "graph",
                ty: TypeSchema::Json,
                comment: "The WorkflowGraph to inspect.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "required_connections",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "One per needed toolkit: { toolkit, status: connected|missing }.",
                required: true,
            }],
        }),
"search_tool_catalog" => Some( ControllerSchema {
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
"get_tool_contract" => Some( ControllerSchema {
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
            }],
        }),
"get_history" => Some( ControllerSchema {
            namespace: "flows",
            function: "get_history",
            description: "List a flow's revision history — prior graph snapshots captured on each \
                          update (capped, newest first). The safety rail behind rollback.",
            inputs: vec![
                id_input("Identifier of the flow whose history to list."),
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max revisions to return (defaults to the retention cap).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "revisions",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Revision snapshots: { id, flow_id, graph, name, require_approval, created_at }.",
                required: true,
            }],
        }),
"rollback" => Some( ControllerSchema {
            namespace: "flows",
            function: "rollback",
            description: "Roll a flow back to a prior revision (restores that revision's graph \
                          through the normal update path — itself snapshotted, so rollback is \
                          undoable). Honours optimistic concurrency via expected_version.",
            inputs: vec![
                id_input("Identifier of the flow to roll back."),
                FieldSchema {
                    name: "revision_id",
                    ty: TypeSchema::String,
                    comment: "The revision (from get_history) to restore.",
                    required: true,
                },
                expected_version_input(),
            ],
            outputs: vec![flow_output()],
        }),
"draft_create" => Some( ControllerSchema {
            namespace: "flows",
            function: "draft_create",
            description: "Create a core-managed draft (a durable, non-live working copy of a graph) \
                          shared by the agent tools and the canvas. Never persists a flow.",
            inputs: vec![
                FieldSchema {
                    name: "name",
                    ty: TypeSchema::String,
                    comment: "Human-readable draft name (carried into the flow on promote).",
                    required: true,
                },
                FieldSchema {
                    name: "graph",
                    ty: TypeSchema::Json,
                    comment: "The (possibly incomplete) WorkflowGraph JSON to hold in the draft.",
                    required: true,
                },
                FieldSchema {
                    name: "flow_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "The saved flow this draft edits, if any (promote → update vs create).",
                    required: false,
                },
                FieldSchema {
                    name: "origin",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Where the draft came from: `chat` | `canvas` | `import`. Defaults to `canvas`.",
                    required: false,
                },
            ],
            outputs: vec![draft_output()],
        }),
"draft_get" => Some( ControllerSchema {
            namespace: "flows",
            function: "draft_get",
            description: "Fetch a draft by id.",
            inputs: vec![id_input("Identifier of the draft to fetch.")],
            outputs: vec![draft_output()],
        }),
"draft_update" => Some( ControllerSchema {
            namespace: "flows",
            function: "draft_update",
            description: "Patch a draft's name/graph/flow_id (any provided field) and bump its \
                          updated_at. Never persists a flow.",
            inputs: vec![
                id_input("Identifier of the draft to update."),
                FieldSchema {
                    name: "name",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "New name, if changing it.",
                    required: false,
                },
                FieldSchema {
                    name: "graph",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "New graph JSON, if changing it.",
                    required: false,
                },
                FieldSchema {
                    name: "flow_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "New linked flow id, if changing it.",
                    required: false,
                },
            ],
            outputs: vec![draft_output()],
        }),
"draft_list" => Some( ControllerSchema {
            namespace: "flows",
            function: "draft_list",
            description: "List all drafts, newest-updated first.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "drafts",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "The drafts (each { id, flow_id?, name, graph, origin, created_at, updated_at }).",
                required: true,
            }],
        }),
"draft_delete" => Some( ControllerSchema {
            namespace: "flows",
            function: "draft_delete",
            description: "Delete a draft by id (idempotent).",
            inputs: vec![id_input("Identifier of the draft to delete.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "`{ id, deleted }` — `deleted` is false if the id was already absent.",
                required: true,
            }],
        }),
"draft_promote" => Some( ControllerSchema {
            namespace: "flows",
            function: "draft_promote",
            description: "Promote a draft into a saved flow through the same create/update gates \
                          (structural validation, forced require_approval floor, born-disabled for \
                          automatic triggers), then delete the draft file. A draft with a flow_id \
                          updates that flow; otherwise it creates a new one.",
            inputs: vec![
                id_input("Identifier of the draft to promote."),
                require_approval_input(),
            ],
            outputs: vec![flow_output()],
        }),
        _ => None,
    }
}
