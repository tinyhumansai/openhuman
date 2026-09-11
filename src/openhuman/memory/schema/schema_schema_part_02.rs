use super::*;

pub(super) fn lookup(function: &str) -> Option<ControllerSchema> {
    match function {
"vault_health_check" => Some( ControllerSchema {
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
        }),
"pipeline_status" => Some( ControllerSchema {
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
                    name: "gate_paused",
                    ty: TypeSchema::Bool,
                    comment: "True while the scheduler gate's live policy is `paused`, \
                              whichever mode is configured (on battery with \
                              `require_ac_power`, CPU pressure, signed out): every \
                              LLM-bound worker, the embed backfill included, is \
                              blocked right now. `is_paused` stays the configured \
                              `off` mode (openhuman#6025).",
                    required: true,
                },
                FieldSchema {
                    name: "gate_pause_reason",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Why the gate is paused: `user_disabled` | `on_battery` | \
                              `cpu_pressure` | `signed_out` | `unknown`. Absent while \
                              running.",
                    required: false,
                },
                FieldSchema {
                    name: "queue_stalled",
                    ty: TypeSchema::Bool,
                    comment: "True when eligible queue work has waited at least six \
                              hours without any job settling (#5324). `status` reads \
                              `degraded` for it; the flag lets a client tell that stall \
                              from the other degradations (openhuman#6025).",
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
                FieldSchema {
                    name: "quarantine",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "openhuman#5820: the most recent corrupt-store quarantine, derived from disk \
                              (`quarantined_at_ms`, `quarantined_path`, `resynced`). Absent when nothing \
                              was quarantined; reported until a chunk lands after it.",
                    required: false,
                },
            ],
        }),
"set_enabled" => Some( ControllerSchema {
            namespace: NAMESPACE,
            function: "set_enabled",
            description: "Toggle Memory Tree auto-sync (#1856 Part 1). \
                Flips `config.scheduler_gate().mode` between `auto` (enabled=true) \
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
        }),
"doctor" => Some( ControllerSchema {
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
        }),
"retry_failed" => Some( ControllerSchema {
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
        }),
"memory_backfill_status" => Some( ControllerSchema {
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
        }),
"smart_walk" => Some( ControllerSchema {
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
        }),
        _ => None,
    }
}
