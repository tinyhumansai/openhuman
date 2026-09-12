use super::*;

pub(super) fn lookup(function: &str) -> Option<ControllerSchema> {
    match function {
"ingest" => Some( ControllerSchema {
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
        }),
"list_chunks" => Some( ControllerSchema {
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
        }),
"get_chunk" => Some( ControllerSchema {
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
        }),
"list_sources" => Some( ControllerSchema {
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
        }),
"search" => Some( ControllerSchema {
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
        }),
"recall" => Some( ControllerSchema {
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
        }),
"entity_index_for" => Some( ControllerSchema {
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
        }),
"chunks_for_entity" => Some( ControllerSchema {
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
        }),
"top_entities" => Some( ControllerSchema {
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
        }),
"chunk_score" => Some( ControllerSchema {
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
        }),
"delete_chunk" => Some( ControllerSchema {
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
        }),
"delete_source" => Some( ControllerSchema {
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
        }),
"wipe_all" => Some( ControllerSchema {
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
        }),
"reset_tree" => Some( ControllerSchema {
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
        }),
"flush_source" => Some( ControllerSchema {
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
        }),
"backfill_connector_trees" => Some( ControllerSchema {
            namespace: NAMESPACE,
            function: "backfill_connector_trees",
            description: "Re-file connector documents stored before the memory-tree \
                          routing fix (#6007) into the tree. Those records are already \
                          embedded in the document store but invisible to tree recall, \
                          the memory graph and the source row's ingest status, and a \
                          re-sync will not recover them: the per-item sync gate treats \
                          them as done. Idempotent — the ingest gate recognises what it \
                          already treed, so a repeated pass writes nothing. EXPENSIVE: \
                          one read and one set of chunk embeddings per document, so \
                          `dry_run` defaults to true and a caller must ask for the write.",
            inputs: vec![
                FieldSchema {
                    name: "dry_run",
                    ty: TypeSchema::Bool,
                    comment: "Report what a pass would examine and write nothing. \
                              Defaults to TRUE — the write is the opt-in.",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::U64,
                    comment: "Documents to examine at most. Omit to leave the bound to \
                              the driver. Resume by calling again; the work is \
                              idempotent, so there is no cursor to carry.",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "executed",
                    ty: TypeSchema::Bool,
                    comment: "Which mode ran: false is the dry-run preview, true a \
                              real pass. The mode, not \"did anything change\" — \
                              `ingested` answers that.",
                    required: true,
                },
                FieldSchema {
                    name: "scanned",
                    ty: TypeSchema::U64,
                    comment: "Documents examined.",
                    required: true,
                },
                FieldSchema {
                    name: "ingested",
                    ty: TypeSchema::U64,
                    comment: "Documents that produced new memory-tree rows.",
                    required: true,
                },
                FieldSchema {
                    name: "already_present",
                    ty: TypeSchema::U64,
                    comment: "Documents the tree already held — what makes a repeated \
                              run readable as \"nothing left to do\".",
                    required: true,
                },
                FieldSchema {
                    name: "skipped",
                    ty: TypeSchema::U64,
                    comment: "Documents left alone rather than filed under a guess.",
                    required: true,
                },
                FieldSchema {
                    name: "more_pending",
                    ty: TypeSchema::Bool,
                    comment: "The pass stopped on its limit with documents unexamined.",
                    required: true,
                },
                FieldSchema {
                    name: "notes",
                    ty: TypeSchema::Json,
                    comment: "Bounded, human-readable reasons behind `skipped`.",
                    required: true,
                },
            ],
        }),
"flush_now" => Some( ControllerSchema {
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
        }),
"graph_export" => Some( ControllerSchema {
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
        }),
"obsidian_vault_status" => Some( ControllerSchema {
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
        }),
        _ => None,
    }
}
