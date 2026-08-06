# Transcript-Derived View — Raw Session Files as Source of Truth

Status: **draft — approved direction, phased implementation**
Branch: `feat/transcript-derived-view` (stacked on `fix/transcript-restore-fidelity`)
Companion: [`conversations-timeline-refactor.md`](conversations-timeline-refactor.md) (Phases 1–2, 4–5 landed; this plan supersedes its Phase 5 hydration story for settled turns)

## Goal

Stop maintaining chat state that must be *synced* with what is live. Derive the settled
transcript from the raw session files (`session_raw/*.jsonl`), and demote every other
store to a cache over that file. Live token streaming is untouched: in-flight turns
render from ephemeral socket-fed state exactly as today; the file is authoritative only
for **settled** turns.

This is the Codex rollout model (one JSONL replayed into both model context and UI,
with an explicit persistence policy) adapted to our layout, plus hermes-agent's
soft-compaction lesson (history is never destroyed, only superseded).

## Why derivation is unsafe today (must fix first)

1. **Destructive compaction.** `agent/harness/session/transcript.rs::write_transcript`
   full-rewrites the file so context reduction deletes earlier turns from disk.
2. **Display data missing from the file**: interrupted partial answers, `request_id`
   turn boundaries, narration items. They exist only in `turn_state` snapshots.
3. **Internal scaffolding in message content**: channel-context prefixes on user
   messages, tool-policy preamble in system content — must be sanitized (or tagged) at
   projection, never shown raw.

## Architecture

```
                     live turn (unchanged)
socket events ──► chatRuntimeSlice (ephemeral) ──► renderer
                                  │ chat_done
                                  ▼ invalidate
settled turns:
session_raw/{root}.jsonl ─┐
session_raw/{root}__sub-*.jsonl ─┴─► core projection (threads.transcript_get)
                                        │  mtime-keyed cache
                                        ▼
                              typed display items ──► renderer (same components)
```

- **Model context** keeps reading the same file via the existing loader, now replaying
  compaction records instead of trusting a rewritten file.
- **`turn_state`** shrinks to live-turn crash recovery (interrupted `streamingText`
  until the interrupted line is appended to the file, then only the in-flight turn).
- The 20-turn retention cap stops being user-visible loss: history comes from the file.

## Phases

### Phase A — append-only transcript (Rust, prerequisite)
`transcript.rs` + harness call sites:
- Replace full-rewrite with **append-only** line writes. Context reduction appends a
  `compaction` record `{ kind: "compaction", replacement_ids | replacement_history }`;
  the model-context loader (`read_transcript` path) replays records to reconstruct the
  post-compaction context; a new display reader returns *all* records.
- Stamp `request_id` on every line of a turn (turn boundary markers); keep `iteration`,
  `ts`, `seq` alignment with the progress-bridge envelope.
- On turn abort/interrupt, append the partial assistant line flagged
  `{ interrupted: true }` so the partial answer is in the file, not only in turn_state.
- Migration: existing files are valid append-only files with zero compaction records —
  no migration needed. Old cores reading new files must skip unknown `kind` lines
  (verify the `_extra` flatten tolerates this; add a version field to `_meta`).
- Tests: compaction round-trip (model context reduced, display history complete),
  interrupted-partial append, request_id stamping, legacy-file read.

### Phase B — projection RPC (Rust)
New `threads.transcript_get(thread_id, {cursor?, limit?})` in the threads domain
(canonical module shape: ops/schemas):
- Resolve root transcript via `find_root_transcript_for_thread`; discover
  `__sub-*.jsonl` children; project into typed display items:
  `user_message | assistant_message | reasoning | tool_call {args, result, status} |
  subagent {id, items} | turn_boundary {request_id} | interrupted_partial`.
- Sanitize scaffolding (channel-context prefix, tool-policy preamble) at projection;
  tag rather than mutate where ambiguity exists.
- Cache: per-thread projection keyed on (file paths, mtimes, lengths); invalidated
  implicitly by key change. No cache writes to disk — pure memory cache.
- Pagination newest-first with cursor; default window sized for one screen.
- Tests: JSON-RPC E2E (`tests/json_rpc_e2e.rs`) — write file, call RPC, assert items;
  subagent merge; sanitization; cache-key invalidation.

### Phase C — frontend switch (TS)
- Thread-open restore path: replace `turn_state_history` hydration for settled turns
  with `transcript_get`; map items onto the existing renderers
  (`PastTurnInsights`/`ToolTimelineBlock`/`ProcessingTranscriptView`, bubbles).
- Live turn: untouched (socket → chatRuntimeSlice). On `chat_done`, drop the live
  turn's ephemeral state and refetch/append the settled projection.
- Keep the turn_state hydration path as fallback behind a flag for one release.
- Tests: restore renders identical item sequence to live for a scripted turn;
  interrupted partial visible; legacy thread (no request_id) fallback.

### Phase D — cleanup
- Demote/remove `turn_state` ring retention (keep live-turn snapshot only), drop the
  `.md` mirror or mark derived-only, delete the fallback flag.

## Non-goals
- No change to the socket streaming protocol or delta handling.
- No SQLite consolidation (sessions.db stays an index; separate discussion).
- Phase 3 of the timeline refactor (dedup consolidation) remains its own track.

## Risks
- **Old-core / new-file compat** — unknown `kind` lines must be skipped, not fatal.
- **File growth** — append-only grows; mitigate later with Codex-style zstd of old
  sessions; out of scope here.
- **Sanitization false positives** — prefer tagging + frontend hiding over deletion.
