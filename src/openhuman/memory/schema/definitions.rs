//! Schema definitions for every `memory_tree` JSON-RPC method.
//!
//! The [`schemas`] function is the single source of truth for each
//! controller's input/output field descriptions. Handlers delegate to
//! [`super::handlers`]; the registry lists are in [`super::registry`].

use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub(crate) const NAMESPACE: &str = "memory_tree";

/// Lookup the [`ControllerSchema`] for a single `memory_tree` function name.
pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "ingest" => ControllerSchema {
            namespace: NAMESPACE,
            function: "ingest",
            description: "Ingest a source into canonical chunks. \
                 Dispatches on `source_kind`; `payload` shape depends on the kind \
                 (chat → ChatBatch, email → EmailThread, document → DocumentInput).",
            inputs: vec![
                FieldSchema {
                    name: "source_kind",
                    ty: TypeSchema::Enum {
                        variants: vec!["chat", "email", "document"],
                    },
                    comment: "Which source kind the payload represents.",
                    required: true,
                },
                FieldSchema {
                    name: "source_id",
                    ty: TypeSchema::String,
                    comment: "Stable logical source id (channel, thread, document id).",
                    required: true,
                },
                FieldSchema {
                    name: "owner",
                    ty: TypeSchema::String,
                    comment: "Optional account / user this content belongs to.",
                    required: false,
                },
                FieldSchema {
                    name: "tags",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Optional tags or labels carried through.",
                    required: false,
                },
                FieldSchema {
                    name: "payload",
                    ty: TypeSchema::Json,
                    comment: "Adapter-specific payload. \
                         chat: {platform, channel_label, messages[]}. \
                         email: {provider, thread_subject, messages[]}. \
                         document: {provider, title, body, modified_at, source_ref}.",
                    required: true,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "source_id",
                    ty: TypeSchema::String,
                    comment: "Logical source id the ingest was scoped to.",
                    required: true,
                },
                FieldSchema {
                    name: "chunks_written",
                    ty: TypeSchema::U64,
                    comment: "Number of chunks persisted after admission.",
                    required: true,
                },
                FieldSchema {
                    name: "chunks_dropped",
                    ty: TypeSchema::U64,
                    comment: "Number of chunks rejected by the admission gate.",
                    required: true,
                },
                FieldSchema {
                    name: "chunk_ids",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "IDs of all chunks persisted after admission.",
                    required: true,
                },
            ],
        },
        "list_chunks" => ControllerSchema {
            namespace: NAMESPACE,
            function: "list_chunks",
            description: "Paginated list of chunks with optional filters by source kind / source id / \
                 entity ids / time window / keyword. Returns chunks plus total match count for \
                 pagination.",
            inputs: vec![
                FieldSchema {
                    name: "source_kinds",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Restrict to one or more source kinds (chat / email / document).",
                    required: false,
                },
                FieldSchema {
                    name: "source_ids",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Restrict to one or more logical source ids.",
                    required: false,
                },
                FieldSchema {
                    name: "entity_ids",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Restrict to chunks indexed against any of these canonical entity ids.",
                    required: false,
                },
                FieldSchema {
                    name: "since_ms",
                    ty: TypeSchema::Option(Box::new(TypeSchema::I64)),
                    comment: "Inclusive lower bound on chunk timestamp (ms since epoch).",
                    required: false,
                },
                FieldSchema {
                    name: "until_ms",
                    ty: TypeSchema::Option(Box::new(TypeSchema::I64)),
                    comment: "Inclusive upper bound on chunk timestamp (ms since epoch).",
                    required: false,
                },
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Substring keyword filter over chunk preview content.",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum rows per page (defaults to 50, capped at 1000).",
                    required: false,
                },
                FieldSchema {
                    name: "offset",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Pagination offset (defaults to 0).",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "chunks",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Ref("Chunk"))),
                    comment: "Page of matching chunks ordered by timestamp DESC.",
                    required: true,
                },
                FieldSchema {
                    name: "total",
                    ty: TypeSchema::U64,
                    comment: "Total number of chunks matching the filter (pre-pagination).",
                    required: true,
                },
            ],
        },
        "get_chunk" => ControllerSchema {
            namespace: NAMESPACE,
            function: "get_chunk",
            description: "Fetch a single chunk by its deterministic id.",
            inputs: vec![FieldSchema {
                name: "id",
                ty: TypeSchema::String,
                comment: "Chunk id (32 hex chars).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "chunk",
                ty: TypeSchema::Option(Box::new(TypeSchema::Ref("Chunk"))),
                comment: "The chunk if found, otherwise null.",
                required: false,
            }],
        },
        "list_sources" => ControllerSchema {
            namespace: NAMESPACE,
            function: "list_sources",
            description: "Distinct (source_kind, source_id) pairs with chunk counts and most-recent timestamps. \
                 `display_name` is computed from the source_id (un-slug + strip user email when known).",
            inputs: vec![FieldSchema {
                name: "user_email_hint",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "When provided, source ids that contain this email get it stripped from \
                          their display name so the UI shows the other party of an email thread.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "sources",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("Source"))),
                comment: "All distinct ingest sources, newest activity first.",
                required: true,
            }],
        },
        "search" => ControllerSchema {
            namespace: NAMESPACE,
            function: "search",
            description: "Keyword LIKE-search over chunk bodies. Cheap, deterministic; useful as a \
                 fallback when semantic recall is unavailable.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Substring to match against chunk content.",
                    required: true,
                },
                FieldSchema {
                    name: "k",
                    ty: TypeSchema::U64,
                    comment: "Maximum chunks to return.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "chunks",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("Chunk"))),
                comment: "Matching chunks ordered by recency.",
                required: true,
            }],
        },
        "recall" => ControllerSchema {
            namespace: NAMESPACE,
            function: "recall",
            description: "Semantic recall — runs the Phase 4 cosine rerank against the query embedding \
                 and returns leaf chunks (not summaries) for UI display.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Free-text query — embedded once and reranked against summary embeddings.",
                    required: true,
                },
                FieldSchema {
                    name: "k",
                    ty: TypeSchema::U64,
                    comment: "Maximum chunks to return.",
                    required: true,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "chunks",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Ref("Chunk"))),
                    comment: "Recalled chunks, sorted in the same order as the rerank.",
                    required: true,
                },
                FieldSchema {
                    name: "scores",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                    comment: "Parallel array of similarity scores (one per chunk).",
                    required: true,
                },
            ],
        },
        "entity_index_for" => ControllerSchema {
            namespace: NAMESPACE,
            function: "entity_index_for",
            description: "Return all canonical entities indexed against a chunk (or summary node) id.",
            inputs: vec![FieldSchema {
                name: "chunk_id",
                ty: TypeSchema::String,
                comment: "Chunk id (32 hex chars).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "entities",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("EntityRef"))),
                comment: "Entities attached to the node, ordered by mention count DESC.",
                required: true,
            }],
        },
        "chunks_for_entity" => ControllerSchema {
            namespace: NAMESPACE,
            function: "chunks_for_entity",
            description: "Return chunk IDs that reference an entity_id (inverse of entity_index_for). \
                 Used by the Memory tab's People/Topics lenses to filter the chunk list.",
            inputs: vec![FieldSchema {
                name: "entity_id",
                ty: TypeSchema::String,
                comment: "Canonical entity id (e.g. `person:Steven Enamakel`, \
                     `email:alice@example.com`).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "chunk_ids",
                ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                comment: "Chunk ids that mention the entity, ordered by recency DESC.",
                required: true,
            }],
        },
        "top_entities" => ControllerSchema {
            namespace: NAMESPACE,
            function: "top_entities",
            description: "Most-frequent canonical entities across the workspace, optionally narrowed by kind.",
            inputs: vec![
                FieldSchema {
                    name: "kind",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Restrict to a single entity_kind (`person`, `email`, `topic`, …).",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::U64,
                    comment: "Maximum rows to return.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "entities",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("EntityRef"))),
                comment: "Top entities, ordered by mention count DESC.",
                required: true,
            }],
        },
        "chunk_score" => ControllerSchema {
            namespace: NAMESPACE,
            function: "chunk_score",
            description: "Score breakdown stored in `mem_tree_score` for one chunk — used by the Memory \
                 tab's 'why was this kept / dropped' panel.",
            inputs: vec![FieldSchema {
                name: "chunk_id",
                ty: TypeSchema::String,
                comment: "Chunk id (32 hex chars).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "breakdown",
                ty: TypeSchema::Option(Box::new(TypeSchema::Ref("ScoreBreakdown"))),
                comment: "Per-signal weight + value array, total, threshold, kept flag, llm_consulted flag.",
                required: false,
            }],
        },
        "delete_chunk" => ControllerSchema {
            namespace: NAMESPACE,
            function: "delete_chunk",
            description: "Purge one chunk plus its score row, entity-index rows, and on-disk .md file. \
                 Idempotent — missing chunk returns deleted=false. Does NOT cascade through \
                 sealed summaries; UIs warn the user.",
            inputs: vec![FieldSchema {
                name: "chunk_id",
                ty: TypeSchema::String,
                comment: "Chunk id to remove.",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "deleted",
                    ty: TypeSchema::Bool,
                    comment: "True when the chunk row was found and removed.",
                    required: true,
                },
                FieldSchema {
                    name: "score_rows_removed",
                    ty: TypeSchema::U64,
                    comment: "Count of rows removed from `mem_tree_score`.",
                    required: true,
                },
                FieldSchema {
                    name: "entity_index_rows_removed",
                    ty: TypeSchema::U64,
                    comment: "Count of rows removed from `mem_tree_entity_index`.",
                    required: true,
                },
            ],
        },
        "delete_source" => ControllerSchema {
            namespace: NAMESPACE,
            function: "delete_source",
            description: "Fully delete one document source by its EXACT source_id: every chunk \
                 plus its score / entity-index / embedding / reembed-skip side rows and chunk \
                 content files, the ingest dedup gates (bare source_id AND versioned \
                 source_id@version), and (when the source becomes fully orphaned) its \
                 source-scoped summary tree — summaries, summary embeddings + reembed-skip, \
                 tree entity-index, buffers, the tree row, and summary content files. Unlike \
                 delete_chunk this cascades, so stale summaries of the deleted source cannot \
                 resurface in recall, and it also finishes legacy partial deletes (chunks already \
                 gone, tree/gate left behind). Exact match only (never a prefix); shared \
                 collection/path_scope trees that summarise multiple documents are left intact. \
                 Idempotent — an unknown source_id returns deleted=false.",
            inputs: vec![FieldSchema {
                name: "source_id",
                ty: TypeSchema::String,
                comment: "Exact source id to remove (e.g. a Telegram note/event/meeting id).",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "deleted",
                    ty: TypeSchema::Bool,
                    comment: "True when the call did real work: chunks were removed OR a stale \
                              orphaned source tree was cleaned (legacy case, chunks_removed=0).",
                    required: true,
                },
                FieldSchema {
                    name: "chunks_removed",
                    ty: TypeSchema::U64,
                    comment: "Number of chunk rows removed for the source.",
                    required: true,
                },
            ],
        },
        "wipe_all" => ControllerSchema {
            namespace: NAMESPACE,
            function: "wipe_all",
            description: "Destructive reset: truncate every mem_tree_* table, remove the \
                          on-disk content folders (raw / wiki / email / chat / document / \
                          legacy summaries) under the workspace memory_tree content root, \
                          and clear every Composio sync-state KV row so the next sync \
                          re-fetches all upstream items. Used by the Memory tab's 'Reset \
                          memory' button.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "rows_deleted",
                    ty: TypeSchema::U64,
                    comment: "Total mem_tree_* rows removed across all tables.",
                    required: true,
                },
                FieldSchema {
                    name: "dirs_removed",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Top-level directories under content_root that were deleted.",
                    required: true,
                },
                FieldSchema {
                    name: "sync_state_cleared",
                    ty: TypeSchema::U64,
                    comment: "Composio sync-state KV rows deleted (cursors + synced-id sets).",
                    required: true,
                },
            ],
        },
        "reset_tree" => ControllerSchema {
            namespace: NAMESPACE,
            function: "reset_tree",
            description: "Wipe summary-tree state but keep chunks + raw archive + sync state, \
                          then re-enqueue every chunk through the extraction pipeline so the \
                          tree rebuilds from scratch. Useful after changing the summariser \
                          backend (e.g. enabling a local LLM) without paying the upstream \
                          re-sync cost.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "tree_rows_deleted",
                    ty: TypeSchema::U64,
                    comment: "Tree-state rows removed (summaries + trees + buffers + jobs).",
                    required: true,
                },
                FieldSchema {
                    name: "chunks_requeued",
                    ty: TypeSchema::U64,
                    comment: "Chunks reset to lifecycle_status = 'pending_extraction'.",
                    required: true,
                },
                FieldSchema {
                    name: "jobs_enqueued",
                    ty: TypeSchema::U64,
                    comment: "extract_chunk jobs enqueued (one per chunk).",
                    required: true,
                },
            ],
        },
        "flush_source" => ControllerSchema {
            namespace: NAMESPACE,
            function: "flush_source",
            description: "Immediately seal one source tree's L0 buffer, bypassing the job \
                          queue. Mutex per source scope so concurrent clicks are serialised. \
                          Returns the number of seal cascades that fired.",
            inputs: vec![FieldSchema {
                name: "source_scope",
                ty: TypeSchema::String,
                comment: "Source tree scope (e.g. `github:org/repo`, `slack:#eng`).",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "tree_scope",
                    ty: TypeSchema::String,
                    comment: "Echo of the source scope.",
                    required: true,
                },
                FieldSchema {
                    name: "seals_fired",
                    ty: TypeSchema::U64,
                    comment: "Number of seal cascades that fired.",
                    required: true,
                },
            ],
        },
        "flush_now" => ControllerSchema {
            namespace: NAMESPACE,
            function: "flush_now",
            description: "Manually trigger the summary-tree build. Enqueues a flush_stale \
                          job with max_age_secs=0 so every L0 buffer force-seals immediately; \
                          the seal worker runs each through the configured (cloud or local) \
                          summariser. Idempotent — same UTC-day dedupe key as the scheduled \
                          flush so spamming the button is safe.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "enqueued",
                    ty: TypeSchema::Bool,
                    comment: "True when a fresh job row was inserted; false when an active \
                              flush job already exists for today.",
                    required: true,
                },
                FieldSchema {
                    name: "stale_buffers",
                    ty: TypeSchema::U64,
                    comment: "Count of L0 buffers that currently qualify for force-seal.",
                    required: true,
                },
            ],
        },
        "graph_export" => ControllerSchema {
            namespace: NAMESPACE,
            function: "graph_export",
            description: "Return either the summary tree (parent→child links between sealed \
                          summary nodes) or the document↔contact graph (chunks linked to \
                          person entities they mention). Includes the absolute path to the \
                          on-disk content root so deep links can point Obsidian at the same \
                          files.",
            inputs: vec![FieldSchema {
                name: "mode",
                ty: TypeSchema::Option(Box::new(TypeSchema::Enum {
                    variants: vec!["tree", "contacts"],
                })),
                comment: "Which graph to return. Defaults to `tree`.",
                required: false,
            }],
            outputs: vec![
                FieldSchema {
                    name: "nodes",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Ref("GraphNode"))),
                    comment: "Summary, chunk, or contact nodes depending on mode.",
                    required: true,
                },
                FieldSchema {
                    name: "edges",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Ref("GraphEdge"))),
                    comment: "Explicit edges. Empty in tree mode (parent_id encodes \
                              edges); chunk→contact mention edges in contacts mode.",
                    required: true,
                },
                FieldSchema {
                    name: "content_root_abs",
                    ty: TypeSchema::String,
                    comment: "Absolute path to <workspace>/memory_tree/content/.",
                    required: true,
                },
            ],
        },
        "obsidian_vault_status" => ControllerSchema {
            namespace: NAMESPACE,
            function: "obsidian_vault_status",
            description: "Best-effort check of whether the memory-tree content root is \
                          already a registered Obsidian vault. `obsidian://open?path=` only \
                          resolves vaults present in Obsidian's obsidian.json registry — it \
                          cannot register a new one — so the Memory tab calls this before \
                          firing the deep link and guides the user to 'Open folder as vault' \
                          when it isn't registered. Never errors; a probe miss reports \
                          registered=false.",
            inputs: vec![FieldSchema {
                name: "obsidian_config_dir",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional override for Obsidian's config directory (where \
                          obsidian.json lives), for non-standard installs \
                          (Flatpak / Snap / portable). Omitted ⇒ probe the standard per-OS \
                          location plus known sandbox paths.",
                required: false,
            }],
            outputs: vec![
                FieldSchema {
                    name: "registered",
                    ty: TypeSchema::Bool,
                    comment: "True when the content root (or an ancestor) is a registered \
                              Obsidian vault, so the deep link will resolve.",
                    required: true,
                },
                FieldSchema {
                    name: "config_found",
                    ty: TypeSchema::Bool,
                    comment: "True when an obsidian.json was found and parsed (Obsidian is \
                              set up). Lets the UI offer add-as-vault vs. install.",
                    required: true,
                },
                FieldSchema {
                    name: "content_root_abs",
                    ty: TypeSchema::String,
                    comment: "Absolute path to <workspace>/memory_tree/content/ — the folder \
                              to add to Obsidian and the deep-link target.",
                    required: true,
                },
            ],
        },
        "vault_health_check" => ControllerSchema {
            namespace: NAMESPACE,
            function: "vault_health_check",
            description: "Consolidated workspace-vault health snapshot for onboarding and \
                          settings. Checks whether <workspace>/memory_tree/content exists, is \
                          readable, and is writable (via temp-file probe), whether Obsidian has \
                          the vault registered, and whether the Memory Tree pipeline is healthy.",
            inputs: vec![FieldSchema {
                name: "obsidian_config_dir",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional override for Obsidian's config directory (where \
                          obsidian.json lives). Omitted ⇒ standard per-OS probe.",
                required: false,
            }],
            outputs: vec![
                FieldSchema {
                    name: "content_root_abs",
                    ty: TypeSchema::String,
                    comment: "Absolute path to <workspace>/memory_tree/content/.",
                    required: true,
                },
                FieldSchema {
                    name: "exists",
                    ty: TypeSchema::Bool,
                    comment: "True when the workspace vault directory exists on disk.",
                    required: true,
                },
                FieldSchema {
                    name: "readable",
                    ty: TypeSchema::Bool,
                    comment: "True when the workspace vault directory can be read.",
                    required: true,
                },
                FieldSchema {
                    name: "writable",
                    ty: TypeSchema::Bool,
                    comment: "True when the vault accepts a create+delete temp-file probe.",
                    required: true,
                },
                FieldSchema {
                    name: "obsidian_registered",
                    ty: TypeSchema::Bool,
                    comment: "True when Obsidian has this folder (or an ancestor) registered \
                              as a vault.",
                    required: true,
                },
                FieldSchema {
                    name: "pipeline_healthy",
                    ty: TypeSchema::Bool,
                    comment: "True when Memory Tree pipeline is not paused and not in error.",
                    required: true,
                },
                FieldSchema {
                    name: "last_sync_ms",
                    ty: TypeSchema::I64,
                    comment: "Epoch ms of the newest chunk timestamp; 0 when empty.",
                    required: true,
                },
            ],
        },
        "pipeline_status" => ControllerSchema {
            namespace: NAMESPACE,
            function: "pipeline_status",
            description: "Aggregated Memory Tree health snapshot (#1856 Part 1). \
                Returns a coarse `status` string (running/paused/syncing/error/idle), \
                an optional human-readable reason, the most-recent chunk timestamp, \
                the total chunk count, the on-disk wiki size in bytes, and per-state \
                job counters from `mem_tree_jobs`. Polled by the Memory Tree status \
                panel; cheap enough to call every couple of seconds.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "status",
                    ty: TypeSchema::Enum {
                        variants: vec![
                            "running", "paused", "syncing", "degraded", "error", "idle",
                        ],
                    },
                    comment: "Coarse, UI-shaped status. Precedence: paused > error > \
                              degraded > syncing > running > idle. `degraded` (#002) = \
                              the pipeline runs but recall/structure is reduced.",
                    required: true,
                },
                FieldSchema {
                    name: "reason",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Human-readable reason for the current status — present \
                              for `paused` (gate mode) and `error` (failed-job count).",
                    required: false,
                },
                FieldSchema {
                    name: "last_sync_ms",
                    ty: TypeSchema::I64,
                    comment: "Epoch ms of the newest chunk timestamp across all \
                              sources; 0 when the store is empty.",
                    required: true,
                },
                FieldSchema {
                    name: "total_chunks",
                    ty: TypeSchema::U64,
                    comment: "Total rows in `mem_tree_chunks`.",
                    required: true,
                },
                FieldSchema {
                    name: "wiki_size_bytes",
                    ty: TypeSchema::U64,
                    comment: "Recursive on-disk size of the `wiki/` sub-tree under the \
                              memory_tree content root. 0 when the directory does not exist yet.",
                    required: true,
                },
                FieldSchema {
                    name: "pipeline_jobs",
                    ty: TypeSchema::Json,
                    comment: "Object with `ready` / `running` / `failed` counters \
                              from `mem_tree_jobs`.",
                    required: true,
                },
                FieldSchema {
                    name: "is_syncing",
                    ty: TypeSchema::Bool,
                    comment: "True when at least one job is in `running` state.",
                    required: true,
                },
                FieldSchema {
                    name: "is_paused",
                    ty: TypeSchema::Bool,
                    comment: "True when scheduler-gate mode is `off`.",
                    required: true,
                },
                FieldSchema {
                    name: "degraded",
                    ty: TypeSchema::Json,
                    comment: "#002 (FR-002/FR-004): object `{ semantic_recall: bool, \
                              structure: bool, cause?: PipelineFailure }`. The pipeline \
                              ran but output quality is reduced — `semantic_recall` when \
                              embeddings were skipped, `structure` when extraction \
                              yielded nothing. `cause` is the single precedence-resolved \
                              failure (structure over semantic_recall) and is OMITTED \
                              when no degradation is active; the recall/structure flags \
                              are tracked independently behind it. The object itself is \
                              always present (serde default). Distinct from a hard `error`.",
                    required: true,
                },
                FieldSchema {
                    name: "first_blocking_cause",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "#002 (FR-004): the single most-urgent typed cause as a \
                              `PipelineFailure` object `{ code, class, remediation_key }`. \
                              A failed job's classified reason wins over a soft \
                              degradation cause. null when healthy. The UI resolves \
                              `remediation_key` and renders it verbatim.",
                    required: false,
                },
                FieldSchema {
                    name: "extraction_coverage",
                    ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
                    comment: "#002 (FR-010): fraction [0.0, 1.0] of chunks with ≥1 \
                              indexed entity. Near 0 with total_chunks > 0 means \
                              extraction produces no structure. `null` when the metric \
                              could not be measured (DB read error) — deliberately \
                              distinct from a genuine `0.0` so a broken measurement is \
                              never misreported as a structure failure.",
                    required: false,
                },
            ],
        },
        "set_enabled" => ControllerSchema {
            namespace: NAMESPACE,
            function: "set_enabled",
            description: "Toggle Memory Tree auto-sync (#1856 Part 1). \
                Flips `config.scheduler_gate.mode` between `auto` (enabled=true) \
                and `off` (enabled=false), persists the change, and hot-reloads \
                the live scheduler-gate so in-flight workers observe the new \
                policy at their next `wait_for_capacity` await. The 20-min \
                Composio fetch loop is NOT paused by this toggle yet — that \
                lands in #1856 Part 2.",
            inputs: vec![FieldSchema {
                name: "enabled",
                ty: TypeSchema::Bool,
                comment: "True ⇒ scheduler-gate mode = auto. False ⇒ mode = off.",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "enabled",
                    ty: TypeSchema::Bool,
                    comment: "Echo of the requested enabled state.",
                    required: true,
                },
                FieldSchema {
                    name: "changed",
                    ty: TypeSchema::Bool,
                    comment: "True when the persisted mode actually flipped; \
                              false for no-ops.",
                    required: true,
                },
                FieldSchema {
                    name: "mode",
                    ty: TypeSchema::String,
                    comment: "New scheduler-gate mode as wire string (`auto` / `off`).",
                    required: true,
                },
            ],
        },
        "doctor" => ControllerSchema {
            namespace: NAMESPACE,
            function: "doctor",
            description: "One-shot Memory pipeline diagnostic (#002). Walks each \
                stage (embeddings config, scheduler gate, job queue, extraction/recall \
                degradation, summary-tree precondition) and returns per-stage health, \
                the single first blocking cause (typed code + i18n remediation key), the \
                degraded snapshot, and counters. Exposed for the agent's self-diagnosis \
                and the CLI; cheap (config + queue counters + degraded flags, no live \
                network probe).",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "healthy",
                    ty: TypeSchema::Bool,
                    comment: "True when no stage is blocking (first_blocking_cause is null).",
                    required: true,
                },
                FieldSchema {
                    name: "stages",
                    ty: TypeSchema::Json,
                    comment: "Ordered array of { stage, ok, failure?, note } — pipeline \
                              order, so the first non-ok stage is the first blocking cause.",
                    required: true,
                },
                FieldSchema {
                    name: "first_blocking_cause",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Typed { code, class, remediation_key, detail? } of the first \
                              non-ok stage; null when healthy. Mirrors \
                              pipeline_status.first_blocking_cause as an explicit Option.",
                    required: false,
                },
                FieldSchema {
                    name: "degraded",
                    ty: TypeSchema::Json,
                    comment: "{ semantic_recall, structure, cause? } degradation snapshot.",
                    required: true,
                },
                FieldSchema {
                    name: "counters",
                    ty: TypeSchema::Json,
                    comment: "{ total_chunks, jobs_ready, jobs_running, jobs_failed, \
                              extraction_coverage: number|null }. extraction_coverage \
                              is the fraction [0,1] of chunks with ≥1 indexed entity; \
                              null when the metric could not be measured (DB error).",
                    required: true,
                },
            ],
        },
        "retry_failed" => ControllerSchema {
            namespace: NAMESPACE,
            function: "retry_failed",
            description: "Requeue every terminally-failed mem_tree_jobs row back to \
                `ready` (#002 FR-011) so jobs that failed under a now-fixed config \
                (e.g. after adding an embeddings key) re-run without re-ingesting \
                source data. Resets the attempt budget and clears the typed failure \
                reason. Manual, on-demand retry — there is no automatic \
                requeue-on-sync yet.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "requeued",
                ty: TypeSchema::U64,
                comment: "Number of failed jobs flipped back to ready for retry.",
                required: true,
            }],
        },
        "memory_backfill_status" => ControllerSchema {
            namespace: NAMESPACE,
            function: "memory_backfill_status",
            description: "Report whether a per-model embedding re-embed \
                backfill (#1574) is in flight. The UI polls this while the \
                re-embed modal is open: semantic recall over not-yet-\
                re-embedded memory is reduced until the chain drains.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "in_progress",
                    ty: TypeSchema::Bool,
                    comment: "True while a re-embed backfill still has work \
                        pending (flag set or a ready/running job).",
                    required: true,
                },
                FieldSchema {
                    name: "pending_jobs",
                    ty: TypeSchema::U64,
                    comment: "Count of reembed_backfill jobs in ready or \
                        running state; 0 with in_progress=false means the \
                        active embedding space is fully covered.",
                    required: true,
                },
            ],
        },
        "smart_walk" => ControllerSchema {
            namespace: NAMESPACE,
            function: "smart_walk",
            description: "Deterministic E2GraphRAG memory retrieval — extracts \
                query entities (spaCy, with regex fallback), routes between \
                entity-graph (local) and dense-summary (global) search with no \
                LLM, and returns ranked evidence hits for a natural-language \
                query.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Natural-language question to answer.",
                    required: true,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::U64,
                    comment: "Max evidence hits to return. Default 10.",
                    required: false,
                },
                FieldSchema {
                    name: "time_window_days",
                    ty: TypeSchema::U64,
                    comment: "Restrict the global/dense branch to the last N days.",
                    required: false,
                },
                FieldSchema {
                    name: "max_hops",
                    ty: TypeSchema::U64,
                    comment: "Entity-graph relatedness hop threshold. Default 2.",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "hits",
                    ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                    comment: "Ranked RetrievalHit evidence (node_id, content, \
                        entities, score, time range, ...).",
                    required: true,
                },
                FieldSchema {
                    name: "total",
                    ty: TypeSchema::U64,
                    comment: "Pre-truncation match count.",
                    required: true,
                },
                FieldSchema {
                    name: "truncated",
                    ty: TypeSchema::Bool,
                    comment: "True when total exceeds the returned hit count.",
                    required: true,
                },
            ],
        },
        _ => ControllerSchema {
            namespace: NAMESPACE,
            function: "unknown",
            description: "Unknown memory_tree controller function.",
            inputs: vec![FieldSchema {
                name: "function",
                ty: TypeSchema::String,
                comment: "Unknown function requested for schema lookup.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}
