# subconscious

The subconscious is OpenHuman's background-awareness layer: a SQLite-backed loop that, on each tick, evaluates a set of user/system tasks against a freshly-built "situation report" (derived from the memory tree) using an LLM, then **acts** on actionable tasks, **escalates** ambiguous/risky ones for user approval, or **noops**. In the same tick the LLM also emits proactive **reflections** (#623) — observation-only cards surfaced on the Intelligence tab. The actual periodic loop is owned by the `heartbeat` domain; this module owns task storage, tick evaluation/execution, escalations, and reflections.

## Responsibilities

- Maintain a list of `SubconsciousTask`s (system-seeded + user-added) in SQLite; seed three default system tasks on init.
- Run a tick: load due tasks → log them `in_progress` → build a situation report → call the configured LLM → execute `act` tasks, create escalations for `escalate`/`UnapprovedWrite`, mark `noop` otherwise → update log entries in place.
- Route the per-tick evaluation and task execution to a local Ollama/LM Studio model or the OpenHuman cloud, based on config (`workload_local_model("subconscious")` / `subconscious_provider`).
- Classify task write-intent via keyword heuristics (`needs_tools` / `needs_agent`); run read-only tasks analysis-only and escalate any recommended write action for approval.
- Emit, cap (`MAX_REFLECTIONS_PER_TICK = 5`), hydrate, and persist proactive reflections; resolve each reflection's `source_refs` into frozen `SourceChunk` snapshots at tick time.
- Manage escalation lifecycle (pending → approved/dismissed); approving executes the task at full permissions.
- Persist `last_tick_at` across restarts so the situation report only feeds the LLM memory-tree rows newer than the last successful tick (dedupe).
- Provide an overlap guard (generation counter) so a newer tick supersedes an in-flight one and discards its results without advancing the cutoff.
- Expose the full task/log/escalation/reflection surface over JSON-RPC.

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | Export-focused (no docstring); re-exports engine, reflection, schemas, source_chunk, and core types. |
| `types.rs` | Domain serde types: `SubconsciousTask`, `TaskSource`, `TaskRecurrence`, `TaskPatch`, `TickDecision`, `TaskEvaluation`, `EvaluationResponse`, `ExecutionResult`, `SubconsciousLogEntry`, `Escalation`/`EscalationPriority`/`EscalationStatus`, `SubconsciousStatus`, `TickResult`. |
| `engine.rs` | `SubconsciousEngine` — the tick loop, evaluation, dispatch (`handle_act`/`handle_escalate`/`handle_noop`), escalation approve/dismiss, provider routing (`resolve_subconscious_route`, `subconscious_provider_unavailable_reason`), LLM response parsing, reflection persistence. |
| `executor.rs` | Per-task execution: routes to local model (text), agentic-v1 full (write-intent), or agentic-v1 analysis-only (read-only). `ExecutionOutcome` (`Completed` / `UnapprovedWrite`), `needs_tools`/`needs_agent` heuristics, 429 retry with backoff, `extract_recommended_action`. |
| `store.rs` | SQLite persistence + DDL for all tables; `with_connection` with busy-timeout + retry (TAURI-RUST-A). Task/log/escalation CRUD, `seed_default_tasks`, `due_tasks`, `compute_next_run` (cron), `get/set_last_tick_at`. |
| `reflection.rs` | `Reflection`, `ReflectionKind`, `ReflectionDraft`; `hydrate_draft`, `apply_cap`, `dedup_key`, `MAX_REFLECTIONS_PER_TICK`. |
| `reflection_store.rs` | SQLite persistence for `subconscious_reflections` + `subconscious_hotness_snapshots`; `list_recent`, `get_reflection`, `add_reflection`, `mark_acted`/`mark_dismissed`, legacy-column + `source_chunks` migrations. |
| `source_chunk.rs` | `SourceChunk` + `resolve_chunks` / `parse_ref` — resolve reflection `source_refs` (`entity:`/`summary:`/`digest:`/…) into frozen content previews (`PREVIEW_MAX_CHARS = 400`). |
| `prompt.rs` | Prompt builders: `build_evaluation_prompt`, `build_text_execution_prompt`, `build_tool_execution_prompt`, `build_analysis_only_prompt`, `load_identity_context` (injects SOUL.md/PROFILE.md). |
| `global.rs` | Engine singleton: `get_or_init_engine`, `bootstrap_after_login`, `stop_heartbeat_loop`, `reset_engine_for_user_switch`. Spawns the `heartbeat` loop and tears it down on logout/user switch. |
| `schemas.rs` | RPC controller schemas + `handle_*` handlers (`subconscious.*`). |
| `decision_log.rs` | In-memory `DecisionLog`/`DecisionRecord` with 24h TTL to avoid re-surfacing the same doc ids. Retained for potential future dedup queries (not wired into the live tick path). |
| `situation_report/` | Situation-report assembly (see below). |
| `*_tests.rs`, `integration_tests.rs` | Sibling and inline test suites. |

### `situation_report/` submodule

| File | Role |
| --- | --- |
| `mod.rs` | `build_situation_report` — assembles sections in priority order under a token budget (env, user identifiers, pending tasks, hotness deltas, sealed summaries, L0 digest, recap window, recent reflections); truncates the tail when over budget. |
| `hotness.rs` | Top entity hotness movers since last tick (`mem_tree_entity_hotness`). |
| `summaries.rs` | Recently-sealed summaries (`mem_tree_summaries`). |
| `digest.rs` | Latest global L0 daily digest body. |
| `query_window.rs` | `query_global` recap window since `last_tick_at`. |
| `reflections.rs` | Renders recent reflections as anti-double-emit context. |

## Public surface

From `mod.rs`:
- `SubconsciousEngine` (`engine`) — `new`, `from_heartbeat_config`, `run`, `tick`, `status`, `add_task`, `approve_escalation`, `dismiss_escalation`.
- `Reflection`, `ReflectionKind`, `MAX_REFLECTIONS_PER_TICK` (`reflection`).
- `SourceChunk` (`source_chunk`).
- Types: `Escalation`, `EscalationStatus`, `SubconsciousLogEntry`, `SubconsciousStatus`, `SubconsciousTask`, `TaskRecurrence`, `TaskSource`, `TickDecision`, `TickResult`.
- `all_subconscious_controller_schemas` / `all_subconscious_registered_controllers` (`schemas`).
- `global` module functions (`get_or_init_engine`, `bootstrap_after_login`, `stop_heartbeat_loop`, `reset_engine_for_user_switch`) are reachable via `subconscious::global::*`.

## RPC / controllers

Namespace `subconscious` (i.e. `openhuman.subconscious_<function>`), all returning `RpcOutcome<T>`:

| Function | Purpose |
| --- | --- |
| `status` | Engine status (read entirely from DB to avoid blocking on the tick mutex). |
| `trigger` | Manually fire a tick (spawned in the background; returns immediately). |
| `tasks_list` | List tasks (optional `enabled_only`). |
| `tasks_add` | Add a task (`title`, optional `source`). |
| `tasks_update` | Patch `title`/`recurrence` (`once` \| `cron:<expr>` \| `pending`)/`enabled`. |
| `tasks_remove` | Delete a task (system tasks cannot be deleted). |
| `log_list` | List execution log entries (optional `task_id`, `limit`). |
| `escalations_list` | List escalations (optional `status`). |
| `escalations_approve` | Approve + execute an escalation. |
| `escalations_dismiss` | Dismiss an escalation without executing. |
| `reflections_list` | List recent reflections (`limit`, `since_ts`). |
| `reflections_act` | Spawn a fresh conversation thread seeded with the reflection body (as an `assistant` message; no LLM turn) and stamp `acted_on_at`; returns `{reflection_id, thread_id}`. |
| `reflections_dismiss` | Set `dismissed_at`. |

Handlers use the shared bounded `load_config_with_timeout()` loader (30s) to avoid stalling the Intelligence-page 3s poll on a slow keychain.

## Agent tools

None. This module owns no `tools.rs` and registers no agent tools.

## Events

No `bus.rs`; the module neither publishes nor subscribes to `DomainEvent`s directly. (The tick loop is driven by the `heartbeat` domain, not the event bus.)

## Persistence

SQLite at `<workspace>/subconscious/subconscious.db` (per-user workspace). Tables (`store.rs` DDL):
- `subconscious_tasks` — task definitions (id, title, source, recurrence, enabled, run times, completed, created_at).
- `subconscious_log` — per-tick execution log (decision `in_progress`/`act`/`escalate`/`noop`/`failed`/`cancelled`/`dismissed`, result, duration).
- `subconscious_escalations` — escalations awaiting user input.
- `subconscious_reflections` — proactive reflections incl. `source_refs`, `source_chunks`, lifecycle timestamps.
- `subconscious_hotness_snapshots` — per-entity previous-tick hotness scores for hotness-delta computation.
- `subconscious_state` — KV table holding `last_tick_at` (restart-durable dedupe cutoff).

`with_connection` runs all DDL + idempotent migrations on every open, with a 5s busy timeout and 3-retry exponential backoff for transient `SQLITE_BUSY`/`SQLITE_LOCKED`.

## Dependencies

- `openhuman::config` — `Config`/`HeartbeatConfig`, provider routing (`workload_local_model`, `subconscious_provider`), bounded loaders, `workspace_dir`.
- `openhuman::heartbeat` — `HeartbeatEngine`; `global.rs` spawns the periodic loop that calls `tick`.
- `openhuman::memory::chat` — `build_chat_provider`/`ChatProvider`/`ChatPrompt` for the per-tick LLM evaluation call.
- `openhuman::inference` — local provider factory + `local::ops::agent_chat` for task execution (executor).
- `openhuman::memory_store` — `MemoryClient`/`MemoryClientRef`, tree types; the engine holds a memory client and the situation report reads tree tables.
- `openhuman::memory_tree` — `retrieval::global::query_global` for the recap-window section.
- `openhuman::memory_conversations` — `ensure_thread`/`append_message` for `reflections_act` thread spawning.
- `openhuman::credentials` — `AuthService`/`APP_SESSION_PROVIDER` to check the OpenHuman cloud session bearer for provider availability.
- `openhuman::scheduler_gate` — `is_signed_out()` gate for the cloud provider.
- `openhuman::composio::providers::profile` — connected-account identifiers for the "Your Identifiers" report section (#1365).
- `openhuman::util` — `floor_char_boundary` for budget-safe truncation.
- `core::all` / `core::{ControllerSchema, FieldSchema, TypeSchema}` + `rpc::RpcOutcome` — RPC controller registration.

## Used by

- `core::all` / `core::jsonrpc` — registers the subconscious controllers into the RPC surface.
- `openhuman::heartbeat::{engine, rpc}` — drives ticks via the engine; `global::bootstrap_after_login` spawns the heartbeat loop.
- `openhuman::agent::harness::session::builder` and `openhuman::context::prompt::SystemPromptBuilder` — inject reflection `source_chunks` as memory context for threads spawned from a reflection.
- `openhuman::agent::prompts` — references subconscious in prompt assembly.
- `openhuman::channels::providers::web` — chat ingress.
- `openhuman::credentials::ops` — login/logout flow triggers `bootstrap_after_login` / `reset_engine_for_user_switch`.

## Notes / gotchas

- **Engine must bootstrap post-login** (`global::bootstrap_after_login`) so `seed_default_tasks` writes to the per-user workspace, not the pre-login global default. `reset_engine_for_user_switch` tears it down on logout/account switch to avoid leaking into the wrong DB.
- **`status` RPC never touches the engine mutex** — it reads counts straight from SQLite, because the engine lock is held for the full tick duration and would otherwise freeze the 3s poll. `consecutive_failures` is therefore reported as `0` from the RPC path (only available from in-memory state).
- **`last_tick_at` is only advanced on success.** Evaluation failure, provider unavailability, or a superseded tick leave the cutoff in place so the next tick re-reads the same window — at the cost of possible re-emitted reflections (there is no insert-time dedupe in `persist_and_surface_reflections`; `dedup_key` exists but is not enforced on insert in the live path).
- **Reflections are observation-only.** The legacy auto-post-into-thread flow was removed; `disposition`/`surfaced_at` columns are dropped via migration and any LLM-emitted `disposition` is ignored by serde.
- **Write-intent gating is heuristic** (`needs_tools`/`needs_agent` keyword matching). Read-only tasks run analysis-only; a `RECOMMENDED ACTION:` line in the output triggers an `UnapprovedWrite` escalation — except on the cloud fallback path for simple text tasks, which deliberately suppresses escalation.
- **`decision_log.rs` is retained but not wired into the live tick** (the comment in `mod.rs` notes it is kept for potential future dedup queries).
- LLM response parsing is best-effort: full envelope → bare evaluations array → all-noop fallback; `extract_json` strips prose around the JSON object/array.
