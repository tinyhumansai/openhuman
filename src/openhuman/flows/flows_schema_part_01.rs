use super::*;

pub(super) fn lookup(function: &str) -> Option<ControllerSchema> {
    match function {
"create" => Some( ControllerSchema {
            namespace: "flows",
            function: "create",
            description: "Create a new saved automation workflow from a tinyflows graph.",
            inputs: vec![
                FieldSchema {
                    name: "name",
                    ty: TypeSchema::String,
                    comment: "Human-readable flow name.",
                    required: true,
                },
                FieldSchema {
                    name: "graph",
                    ty: TypeSchema::Json,
                    comment:
                        "A tinyflows WorkflowGraph (nodes + edges); validated and migrated on save.",
                    required: true,
                },
                require_approval_input(),
                strict_input(),
            ],
            outputs: vec![flow_output()],
        }),
"duplicate" => Some( ControllerSchema {
            namespace: "flows",
            function: "duplicate",
            description: "Duplicate a saved flow: create an independent copy of its graph under a \
                          new id, with the name suffixed \" (copy)\". The copy is created DISABLED \
                          and is NOT schedule/trigger-bound, so it never immediately fires — the \
                          user enables it explicitly once reviewed. Run history does not carry over.",
            inputs: vec![id_input("Identifier of the flow to duplicate.")],
            outputs: vec![flow_output()],
        }),
"validate" => Some( ControllerSchema {
            namespace: "flows",
            function: "validate",
            description: "Validate a tinyflows graph without saving it: reports structural \
                          validity plus non-fatal warnings (e.g. a trigger kind that does not \
                          fire automatically yet).",
            inputs: vec![FieldSchema {
                name: "graph",
                ty: TypeSchema::Json,
                comment: "A tinyflows WorkflowGraph (nodes + edges) to validate and migrate.",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "valid",
                    ty: TypeSchema::Bool,
                    comment: "True when the graph is structurally valid.",
                    required: true,
                },
                FieldSchema {
                    name: "errors",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Structural validation errors; empty when `valid`.",
                    required: true,
                },
                FieldSchema {
                    name: "warnings",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Non-fatal warnings (e.g. an unfired trigger kind); the graph is \
                              still saveable/enable-able.",
                    required: true,
                },
            ],
        }),
"import" => Some( ControllerSchema {
            namespace: "flows",
            function: "import",
            description: "Import a workflow definition WITHOUT saving it: parse a native tinyflows \
                          graph or an n8n workflow export, migrate + validate it, and return the \
                          normalized WorkflowGraph plus non-fatal import warnings. The caller opens \
                          the result on the canvas as a draft and Saves via the normal gate — \
                          import never persists or enables anything.",
            inputs: vec![
                FieldSchema {
                    name: "graph",
                    ty: TypeSchema::Json,
                    comment: "The workflow JSON to import: a tinyflows WorkflowGraph (native) or \
                              an n8n workflow export.",
                    required: true,
                },
                FieldSchema {
                    name: "format",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Enum {
                        variants: vec!["native", "n8n", "auto"],
                    })),
                    comment: "Source format: `native` (tinyflows), `n8n`, or `auto` (default — \
                              detect by shape).",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "graph",
                    ty: TypeSchema::Json,
                    comment: "The normalized, migrated + validated WorkflowGraph, ready to open \
                              as an editable draft.",
                    required: true,
                },
                FieldSchema {
                    name: "warnings",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Non-fatal import warnings (unmapped n8n node types, untranslated \
                              expressions, a synthesized/demoted trigger). Empty for a clean \
                              native import.",
                    required: true,
                },
            ],
        }),
"get" => Some( ControllerSchema {
            namespace: "flows",
            function: "get",
            description: "Load one saved flow by id.",
            inputs: vec![id_input("Identifier of the flow to load.")],
            outputs: vec![flow_output()],
        }),
"list" => Some( ControllerSchema {
            namespace: "flows",
            function: "list",
            description: "List all saved flows.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "flows",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("Flow"))),
                comment: "Flows currently stored in the workspace.",
                required: true,
            }],
        }),
"list_connections" => Some( ControllerSchema {
            namespace: "flows",
            function: "list_connections",
            description: "List the connection sources a flow node's `connection_ref` can attach \
                          to: Composio connected accounts (kind `composio`) and stored HTTP \
                          credentials (kind `http`). Returns only non-secret metadata — ids, \
                          display labels, kind, and (for Composio) the connected account's own \
                          `platform_user_id` — never any secret material (OAuth/bearer tokens, \
                          passwords, and API keys stay server-side and are injected only at \
                          execution time).",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "connections",
                ty: TypeSchema::Array(Box::new(TypeSchema::Object {
                    fields: flow_connection_fields(),
                })),
                comment: "Resolvable connections for the flows picker (composio + http), \
                          secret-free.",
                required: true,
            }],
        }),
"update" => Some( ControllerSchema {
            namespace: "flows",
            function: "update",
            description: "Update a saved flow's name and/or graph; re-validates before persisting.",
            inputs: vec![
                id_input("Identifier of the flow to update."),
                FieldSchema {
                    name: "name",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "New name, if changing it.",
                    required: false,
                },
                FieldSchema {
                    name: "graph",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Replacement WorkflowGraph, if changing it.",
                    required: false,
                },
                require_approval_input(),
                strict_input(),
                expected_version_input(),
            ],
            outputs: vec![flow_output()],
        }),
"delete" => Some( ControllerSchema {
            namespace: "flows",
            function: "delete",
            description: "Delete a saved flow by id.",
            inputs: vec![id_input("Identifier of the flow to delete.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "id",
                            ty: TypeSchema::String,
                            comment: "Identifier that was requested for removal.",
                            required: true,
                        },
                        FieldSchema {
                            name: "removed",
                            ty: TypeSchema::Bool,
                            comment: "True when the flow was removed.",
                            required: true,
                        },
                    ],
                },
                comment: "Removal result payload.",
                required: true,
            }],
        }),
"set_enabled" => Some( ControllerSchema {
            namespace: "flows",
            function: "set_enabled",
            description: "Enable or disable a saved flow.",
            inputs: vec![
                id_input("Identifier of the flow to toggle."),
                FieldSchema {
                    name: "enabled",
                    ty: TypeSchema::Bool,
                    comment: "New enabled state.",
                    required: true,
                },
            ],
            outputs: vec![flow_output()],
        }),
"run" => Some( ControllerSchema {
            namespace: "flows",
            function: "run",
            description:
                "Run a saved flow to completion (or until it pauses on a human-approval gate).",
            inputs: vec![
                id_input("Identifier of the flow to run."),
                FieldSchema {
                    name: "input",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Trigger payload seeded into the run; defaults to null.",
                    required: false,
                },
                FieldSchema {
                    name: "inputs",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Values for the flow's declared workflow inputs, keyed by name \
                              (read the flow's `graph.inputs` for the declarations). Missing \
                              required values, wrong types, and undeclared names are rejected \
                              before the run starts. Distinct from `input`, which is the \
                              free-form trigger payload.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: run_output_fields(),
                },
                comment: "Run outcome payload.",
                required: true,
            }],
        }),
"run_detached" => Some( ControllerSchema {
            namespace: "flows",
            function: "run_detached",
            description: "Start a saved flow WITHOUT waiting for it to finish: validates + \
                          compile-checks the flow, registers the run, inserts its `running` row, \
                          and returns the run id immediately. Use this from any UI that wants to \
                          show live per-node progress (`flow:run_progress`) or that must not block \
                          on a run that can take minutes — poll `flows_get_run(run_id)` or the \
                          progress event stream for completion. `run` remains available for callers \
                          that genuinely want to await the final result.",
            inputs: vec![
                id_input("Identifier of the flow to run."),
                FieldSchema {
                    name: "input",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Trigger payload seeded into the run; defaults to null.",
                    required: false,
                },
                FieldSchema {
                    name: "inputs",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Values for the flow's declared workflow inputs, keyed by name                               (read the flow's `graph.inputs` for the declarations). Validated                               synchronously, so a bad set is refused here rather than surfacing                               later as a failed background run. Distinct from `input`, which is                               the free-form trigger payload.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: run_detached_output_fields(),
                },
                comment: "Immediate start-of-run payload — returned as soon as the run is \
                          registered, without waiting for it to finish.",
                required: true,
            }],
        }),
"resume" => Some( ControllerSchema {
            namespace: "flows",
            function: "resume",
            description: "Resume a flow run paused at a human-in-the-loop approval gate, \
                           continuing from its durable checkpoint.",
            inputs: vec![
                id_input("Identifier of the flow to resume."),
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment:
                        "The checkpoint thread id returned by `flows_run` / a prior `flows_resume`.",
                    required: true,
                },
                FieldSchema {
                    name: "approvals",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Node ids being approved; defaults to an empty list.",
                    required: false,
                },
                FieldSchema {
                    name: "rejections",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Node ids being denied; each routes to its `error` port (or fails \
                              the run if it has none). Defaults to an empty list.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: run_output_fields(),
                },
                comment: "Resume outcome payload (same shape as `run`'s).",
                required: true,
            }],
        }),
"cancel_run" => Some( ControllerSchema {
            namespace: "flows",
            function: "cancel_run",
            description: "Cancel a flow run: settle it to a terminal `cancelled` status, abort \
                          the in-flight run task if one is executing, and drop its durable \
                          checkpoint so it can't be resumed.",
            inputs: vec![FieldSchema {
                name: "run_id",
                ty: TypeSchema::String,
                comment: "Identifier of the run to cancel (== its checkpoint thread id).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "run_id",
                            ty: TypeSchema::String,
                            comment: "Identifier of the run that was cancelled.",
                            required: true,
                        },
                        FieldSchema {
                            name: "cancelled",
                            ty: TypeSchema::Bool,
                            comment:
                                "True once the run is cancelled or its cancellation requested.",
                            required: true,
                        },
                        FieldSchema {
                            name: "was_in_flight",
                            ty: TypeSchema::Bool,
                            comment:
                                "True when a live run task was signalled to abort; false when \
                                      a parked/stale run row was settled directly.",
                            required: true,
                        },
                    ],
                },
                comment: "Cancellation result payload.",
                required: true,
            }],
        }),
"list_runs" => Some( ControllerSchema {
            namespace: "flows",
            function: "list_runs",
            description: "List the most recent runs for a flow, newest first.",
            inputs: vec![
                id_input("Identifier of the flow whose runs to list."),
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum number of runs to return; defaults to 20.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "runs",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("FlowRun"))),
                comment: "Persisted run records for this flow, newest first.",
                required: true,
            }],
        }),
"list_all_runs" => Some( ControllerSchema {
            namespace: "flows",
            function: "list_all_runs",
            description: "List the most recent runs across all flows, newest first.",
            inputs: vec![FieldSchema {
                name: "limit",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Maximum number of runs to return; defaults to 100.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "runs",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("FlowRun"))),
                comment: "Persisted run records across all flows, newest first.",
                required: true,
            }],
        }),
"get_run" => Some( ControllerSchema {
            namespace: "flows",
            function: "get_run",
            description: "Load one persisted flow run record by its (checkpoint thread) id.",
            inputs: vec![FieldSchema {
                name: "run_id",
                ty: TypeSchema::String,
                comment: "Identifier of the run to load (== its checkpoint thread id).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "run",
                ty: TypeSchema::Ref("FlowRun"),
                comment: "The persisted run record.",
                required: true,
            }],
        }),
"prune_runs" => Some( ControllerSchema {
            namespace: "flows",
            function: "prune_runs",
            description: "Manually prune a flow's run history down to the retention cap, deleting \
                          only terminal runs (completed/failed/cancelled) outside the newest-N \
                          window. Never removes a running or pending_approval run. Pruning also \
                          happens automatically on every new run; this is an explicit on-demand \
                          sweep.",
            inputs: vec![id_input("Identifier of the flow whose run history to prune.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "flow_id",
                            ty: TypeSchema::String,
                            comment: "Identifier of the flow whose runs were pruned.",
                            required: true,
                        },
                        FieldSchema {
                            name: "pruned",
                            ty: TypeSchema::U64,
                            comment: "Number of run records removed.",
                            required: true,
                        },
                        FieldSchema {
                            name: "kept",
                            ty: TypeSchema::U64,
                            comment: "The retention cap (most-recent runs kept).",
                            required: true,
                        },
                    ],
                },
                comment: "Prune result payload.",
                required: true,
            }],
        }),
"build" => Some( ControllerSchema {
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
"build_cancel" => Some( ControllerSchema {
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
"discover" => Some( ControllerSchema {
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
"list_suggestions" => Some( ControllerSchema {
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
"dismiss_suggestion" => Some( ControllerSchema {
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
        _ => None,
    }
}
