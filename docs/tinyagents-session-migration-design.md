# TinyAgents Session Migration Design

Date: 2026-07-01

Status: Phase 1 implemented (2026-07-01) in `src/openhuman/session_import/`
as the `openhuman.session_import_run` controller
(`openhuman-core session-import run`). Phases 2–4 (read-side shadow, cutover,
retirement) are not started. Implementation deviations from the original
sketch are marked "as built" below.

Goal: a one-time, idempotent migration of persisted OpenHuman session data —
transcript JSONL, legacy Markdown transcripts, and run-ledger/sub-agent rows —
into TinyAgents store/journal records, so new internals can read by TinyAgents
`thread_id` / `run_id` / stream offset while legacy surfaces keep answering by
OpenHuman session key.

## Source inventory (what exists on disk today)

All facts verified against the current checkout (TinyAgents 1.3 pinned with the
`sqlite` feature enabled).

### 1. Transcript JSONL (source of truth)

- Path: `{workspace}/session_raw/{stem}.jsonl`
  (writer/reader: `src/openhuman/agent/harness/session/transcript.rs`).
- Legacy layout still readable: `session_raw/{DDMMYYYY}/{stem}.jsonl`
  (pre-0.53.4 date folders). A layout migration already exists
  (`session/migration.rs`, marker `state/migrations/session_layout_v1.done`).
- Stem encodes identity and lineage:
  - root session: `{unix_ts}_{agent_id}`;
  - sub-agent: `{parent_chain}__{unix_ts}_{agent_id}` — the `__` chain is the
    only parent/child link on disk; there is no pointer file.
- Line 1 is a `_meta` header (`MetaPayload`): `agent`, optional `agent_id` /
  `agent_type` / `provider` / `model` / `thread_id` / `task_id`, `dispatcher`,
  `created`, `updated`, `turn_count`, `input_tokens`, `output_tokens`,
  `cached_input_tokens`, `charged_amount_usd`.
- Remaining lines are `MessageLine`s: required `role` + `content`; optional
  `id`, `extra_metadata`, `provider`, `model`, `usage` (`input`, `output`,
  `cached_input`, `context_window`, `cost_usd`), `reasoning_content`,
  `tool_calls` (`id`, `name`, `arguments` as raw JSON string, optional
  `extra_content`), `iteration`, `ts` (RFC-3339). Only the last assistant
  message of each turn carries the per-turn fields.
- Tool-call encoding varies by the `_meta.dispatcher` value:
  - native: structured `tool_calls` array on the assistant line;
  - XML / P-format: markup (`<tool_call>…</tool_call>`, `name[a|b]`) embedded
    verbatim in `content` — never re-parsed on resume today.

### 2. Markdown transcripts

- Human-readable companion: `{workspace}/sessions/{YYYY_MM_DD}/{stem}.md`
  (legacy `sessions/{DDMMYYYY}/`). Never read back except by the one-release
  legacy reader `read_transcript_legacy_md()` (HTML-comment `<!--MSG …-->`
  format). Treat `.md` as a **fallback source only** when a stem has no JSONL.

### 3. Run ledger (SQLite `{workspace}/session_db/sessions.db`)

- `agent_runs`: id, kind (subagent | worker_thread | background_agent |
  team_member | workflow_child), parent_run_id, parent_thread_id, agent_id,
  status, worker_thread_id, task ids, checkpoint refs, metadata, timestamps.
- `run_events` (`run_id` + `sequence` → `event_type`, `payload_json`) and
  `run_telemetry` (per-run token/cost roll-up).
- `graph_checkpoints` (written by `SqlRunLedgerCheckpointer`,
  `src/openhuman/tinyagents/checkpoint.rs`): seq, thread_id, checkpoint_id,
  run_id, record_json (a full tinyagents `Checkpoint<State>`), created_at.
- **No table stores the transcript stem/path.** Ledger row ↔ transcript file
  correlation is by convention via `thread_id` (+ `task_id`).

### 4. Durable sub-agent sessions (JSON blob)

- `{workspace}/.openhuman/subagent_sessions.json`: a single pretty-printed
  `Vec<DurableSubagentSession>` — `subagentSessionId`, `parentSession`
  (parent session_key), `parentThreadId`, `workerThreadId`, `agentId`,
  toolkit/model/sandbox/action-root selector fields, `status`, `reusable`,
  inline `latestHistory` message mirror, timestamps.

## Target shape (TinyAgents 1.3 primitives)

Use the crate's `harness::store` as the substrate — no new storage layer:

- `AppendStore` / `JsonlAppendStore`: one line per `StoreRecord { offset,
  value, created_at_ms }`, offset = line index. Streams live under a store
  root as `<stream>.jsonl`.
- `Store` / `FileStore`: key-value records as `<namespace>/<key>.json`.

Layout as built under `{workspace}/tinyagents_store/` (`kv/` holds the
`FileStore`, `journal/` the `JsonlAppendStore`). Two constraints reshaped the
original sketch:

- TinyAgents store/stream names are **slash-free** (ASCII alphanumerics plus
  `-_.` — the crate's path-traversal guard), so the `thread/{id}/messages`
  shape is impossible; names are dot-separated.
- Journals are **per session, not per thread**: multiple transcript files can
  share one `_meta.thread_id`, and appending them into a shared stream would
  interleave sessions. The descriptor carries `thread_id`, so thread-level
  views remain a projection.

| Record                        | Primitive     | Stream / key                                     |
| ----------------------------- | ------------- | ------------------------------------------------ |
| Message journal (per session) | `AppendStore` | stream `session.{session_key}.messages`          |
| Session descriptor            | `Store`       | ns `sessions`, key `{session_key}`               |
| Item idempotency ledger       | `Store`       | ns `migration_items`, key `sha256(source path)`  |
| Global run marker             | `Store`       | ns `migrations`, key `session_import_v1`         |

Run event journals and run descriptors were dropped from v1 (see the resolved
open questions at the end): run events stay queryable in SQLite and belong to
the P2 journal-canonicalization work.

Descriptor records carry the compatibility mapping both directions:

```json
// ns sessions, key {session_key}   (session_key = transcript stem)
{
  "session_key": "1719800000_orchestrator",
  "parent_session_key": null,            // from the __ stem chain
  "thread_id": "…",                       // _meta.thread_id or imported-{stem}
  "thread_id_synthesized": false,
  "task_id": "…",                         // from _meta.task_id (nullable)
  "run_ids": ["…"],                       // joined from agent_runs via thread_id
  "stream": "session.1719800000_orchestrator.messages",
  "dispatcher": "native",
  "agent_name": "…", "agent_id": "…", "agent_type": "…",
  "provider": "…", "model": "…",
  "created": "…", "updated": "…", "turn_count": 1,
  "usage": { "input": 0, "output": 0, "cached_input": 0, "cost_usd": 0.0 },
  "source": { "jsonl": "session_raw/….jsonl", "md": null },
  "import": { "version": 1, "imported_at": "…", "warnings": 0 }
}
```

Message journal values (as built) are full-fidelity records of what
`read_transcript()` returns — `{id?, role, content, extra_metadata?}`, where
`extra_metadata` carries the reconstructed `openhuman_turn_usage` block
(`iteration`, `reasoning_content`, per-turn `usage`, `tool_calls` including
`extra_content`). `ChatMessage` itself marks `id`/`extra_metadata`
`skip_serializing`, so the journal defines its own record type
(`JournalMessage`). Projection into the tinyagents `harness::message::Message`
model is left to the read side.

### Lineage keys

- `thread_id`: taken from `_meta.thread_id` when present; when absent (old
  files), synthesize `imported:{session_key}` so every migrated session has a
  stable thread stream. Record the synthesis in the descriptor.
- Parent/child: derive from the `__` stem chain and cross-check against
  `agent_runs.parent_thread_id` / `subagent_sessions.json`; disagreements are
  warnings, stem chain wins (it is the write-time truth).
- `root_run_id`: tinyagents carries it in graph types but OpenHuman's
  `graph_checkpoints` schema drops it. The importer sets `root_run_id` on run
  descriptors by walking `agent_runs.parent_run_id` to the root. (Separately,
  adding a `root_run_id` column to `graph_checkpoints` is a small schema
  follow-up — tracked in the audit, not part of this migration.)

## Tool-call normalization

- Native-dispatcher transcripts: `tool_calls` arrays map 1:1 onto tinyagents
  `ToolCall` records (`arguments` parsed from the raw JSON string; parse
  failure → keep as string + warning).
- XML / P-format transcripts: **do not re-parse in v1.** The markup stays
  verbatim in message content, exactly as the live resume path treats it
  today; the descriptor records `"dispatcher": "xml" | "pformat"` so a later
  pass (or read-side shim) can re-extract structured calls using the existing
  `ToolDispatcher` parsers if ever needed. Re-parsing at import time is high
  risk (P-format needs the positional-arg registry of the tool set as it
  existed then) for no current consumer.

## Idempotency and observability

Follow the proven `session_layout_v1` pattern, plus per-item ledger entries:

- Global marker: `Store` record `migrations/session_import_v1` with run
  timestamp, counters, and tool version. Present → skip scan entirely
  (bypassed by `--only`, `--force`, and dry runs).
- Per-source ledger: each imported stem writes
  `migration_items/{sha256(workspace-relative source path)}` with source size
  + mtime. Re-runs (e.g. after a crash) skip completed items; a changed
  size/mtime re-imports and overwrites that item's records (only that
  session's stream file is reset and rewritten).
- Never mutate or delete sources. `session_raw/`, `sessions/`, `sessions.db`,
  and `subagent_sessions.json` remain untouched; legacy readers keep working
  until parity is proven.
- Surface (as built): the `openhuman.session_import_run` controller —
  `openhuman-core session-import run` / RPC — with `dry_run`, `only`
  (stem glob), `force`, `verbose`, and `workspace` (dir override) params.
  - dry-run prints the per-file plan (stem → thread stream, message count,
    dialect, warnings) and writes nothing;
  - real run emits a summary: files scanned / imported / skipped / failed,
    messages written, warnings list; grep-friendly `[session-import]` log
    prefix on every line.
- Failure policy: per-file errors are warnings (matching
  `migrate_session_layout_if_needed`); the command never aborts the batch and
  never blocks core startup — it is an explicit command, not a boot hook, in
  v1. Wiring it into startup comes only after parity tests.

## Fixture matrix (required before implementation is "done")

Implemented in `src/openhuman/session_import/ops_tests.rs` (18 tests covering
every row below, plus dry-run purity and sources-untouched assertions).

Golden-file tests over real captured shapes:

1. Current flat `session_raw/{ts}_{agent}.jsonl`, native dispatcher.
2. Legacy date-folder `session_raw/{DDMMYYYY}/…` (pre-layout-migration).
3. Legacy Markdown-only session (no JSONL twin) via the `<!--MSG-->` reader.
4. Sub-agent stems, including a two-level `a__b__c` chain.
5. XML-dialect transcript (markup-in-content preserved byte-for-byte).
6. P-format transcript (ditto).
7. Assistant line with `tool_calls` incl. `extra_content` (Gemini
   thought-signature passthrough).
8. Malformed files: missing `_meta` first line, truncated last line, empty
   file, unparseable message line (skip + warning, matching the current
   reader's tolerance).
9. `_meta` without `thread_id` (synthesized thread id path).
10. Ledger cross-check: `agent_runs` row whose `parent_thread_id` disagrees
    with the stem chain (warning path).
11. Idempotency: import → re-run (all skipped) → touch one source → only that
    item re-imports.

Parity assertion: for every fixture, reading the migrated thread stream back
and projecting it into `ChatMessage` history must equal what
`read_transcript()` returns for the source file (same messages, same
`openhuman_turn_usage` reconstruction).

## Phasing

1. **Importer + CLI + fixtures** — done (`src/openhuman/session_import/`):
   write-only into `tinyagents_store/`, sources untouched, nothing reads the
   new records yet.
2. **Read-side shadow**: run-inspection surfaces read both and diff-log
   mismatches (behind a debug flag).
3. **Cutover**: new internals read TinyAgents records; legacy readers stay as
   compatibility projections keyed by `session_key`.
4. **Retirement**: delete legacy readers once telemetry shows no shadow
   mismatches (separate decision, out of scope here).

## Open questions (resolved in v1)

- `run_events` / `run_telemetry` journaling: **not in v1** — transcripts +
  descriptors only. Run events stay queryable in SQLite; P2 (journal
  canonicalization) owns that surface.
- Store root: **`{workspace}/tinyagents_store/`** (`kv/` + `journal/`).
  Living under the workspace means the fail-closed
  `is_workspace_internal_path` guard keeps agent tools from writing here —
  desirable.
- `subagent_sessions.json` `latestHistory` mirrors: **dropped** — they are a
  cache of the same messages the child's own transcript carries; the child
  stem imports as its own session.
