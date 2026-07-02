# Stage 3 — Core ingest + session state (`src/openhuman/orchestration/`)

## Goal command

> Create a new Rust domain `src/openhuman/orchestration/` that ingests tiny.place Signal DMs,
> recognizes `HarnessSessionEnvelope` v1 payloads (stage 1), and maintains durable per-session
> state (master / subconscious / session chat windows). This is the "channel ingestion" boundary of
> the split-brain graph: it normalizes heterogeneous DM traffic into typed graph inputs and
> persists a chat-window model the RPC/UI layer (stage 7) can read directly — replacing the string
> heuristics currently in `TinyPlaceOrchestrationTab.tsx`.

## Read first

- `src/openhuman/tinyplace/` — `mod.rs` (architecture), `schemas.rs` (`handle_tinyplace_messages_list`,
  `handle_tinyplace_signal_send_message`), `signal_store.rs`, `streams.rs`, `state.rs`.
- `src/openhuman/subconscious/store.rs` — SQLite-per-domain store pattern.
- `src/core/event_bus/` — `DomainEvent`, `publish_global`/`subscribe_global`, domain `bus.rs` convention.
- `tiny.place/sdk/typescript/src/types/session-envelope.ts` (after stage 1) — the wire schema.
- Canonical module shape table in `CLAUDE.md`.

## Deliverables

1. **Domain skeleton** (canonical shape): `orchestration/{mod.rs, types.rs, store.rs, ops.rs,
   schemas.rs, bus.rs, ingest.rs}` + inline tests. Wire `all_controller_schemas` into
   `src/core/all.rs` (schemas themselves land in stage 7; register the namespace now).
2. **`types.rs`**: Rust mirror of `HarnessSessionEnvelope` (serde, `#[serde(tag = "v")]`-style
   versioning tolerant of unknown fields), plus:
   - `ChatKind { Master, Subconscious, Session }`
   - `OrchestrationSession { session_id, source (Codex|ClaudeCode|Other), label, workspace,
     last_seq, created_at, last_message_at, active }`
   - `OrchestrationMessage { id, session_id, chat_kind, role, body, timestamp, encrypted, seq }`
3. **`ingest.rs`**: subscribe to incoming tiny.place DMs. Preferred seam: a `DomainEvent`
   published by the tinyplace domain when a Signal DM is received/decrypted (add
   `DomainEvent::TinyplaceDmReceived { from, envelope_json, … }` in the tinyplace stream/inbox
   path — `streams.rs` websocket handler and the poll path both publish). Fallback if streams are
   unavailable: a poll loop calling the existing messages-list op with a cursor. Classification:
   - body parses as `HarnessSessionEnvelope` → `ChatKind::Session`, keyed by `sessionId`.
   - DM from the agent's own subconscious identity/thread marker → `ChatKind::Subconscious`.
   - everything else from the owner/human counterpart → `ChatKind::Master`.
   Idempotent by `(session_id, seq)` / message id — re-ingest must not duplicate.
4. **`store.rs`**: SQLite at `<workspace>/orchestration/orchestration.db` — tables `sessions`,
   `messages` (indexed by session + timestamp), `kv`. Retention: prune messages beyond N=2000 per
   session (configurable). Message bodies stored decrypted here are workspace-internal — protected
   by `is_workspace_internal_path`.
5. **`bus.rs`**: `OrchestrationIngestSubscriber` (`name() = "orchestration::ingest"`), registered
   at startup next to the other bus registrations; also publish
   `DomainEvent::OrchestrationSessionMessage` after persist, so stage 4 (front-end graph) and
   stage 7 (UI socket push) can both react without coupling.
6. **Logging**: `[orchestration]` prefix; log envelope seq/session/kind on ingest entry/exit,
   classification decisions, dedupe skips, parse failures (body **never** logged).

## Tasks

1. Scaffold domain + wire `mod.rs`/`all.rs`; add config knob `[orchestration] enabled = true`
   (schema in `src/openhuman/config/schema/`, follow `scheduler_gate.rs` pattern).
2. Implement types + envelope parsing with fixture tests (valid v1, unknown v, junk body → Master).
3. Implement store + migrations + retention, unit-tested with tempdir DBs.
4. Add the `TinyplaceDmReceived` event publication in `tinyplace::streams`/inbox ops; implement
   `OrchestrationIngestSubscriber`; startup registration.
5. Poll-fallback ingest with cursor in `kv` (used when the Signal stream is down), behind the same
   dedupe.
6. Tests: end-to-end ingest of a synthetic envelope sequence → sessions/messages rows; dedupe on
   replay; classification matrix.

## Acceptance criteria

- Feeding N stage-1 envelopes (mixed sessions, out-of-order seq, duplicates) through the
  subscriber yields correctly bucketed, deduped `sessions`/`messages` rows.
- Non-envelope DMs land in the Master window; nothing crashes on malformed bodies.
- `cargo check` + `pnpm test:rust` green; new code ≥80% line coverage on the diff.
- No message bodies or seeds in logs at any level.
