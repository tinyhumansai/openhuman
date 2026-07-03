# 04.1 — Live turns write tinyagents records

Today new turns append to `session_raw/*.jsonl` via `transcript.rs`;
the tinyagents store only receives one-time imports.

## Steps

1. Add a session store handle (same `FileStore`/`JsonlAppendStore` layout as
   `session_import`: `{workspace}/tinyagents_store/{kv,journal}`) to the
   session shell; register it on `RunContext.stores` in
   `assemble_turn_harness`.
2. After each turn, append the turn's messages to
   `session.{stem}.messages` and upsert the session descriptor (ns
   `sessions`) — dual-write alongside the legacy JSONL append in
   `session/turn/session_io.rs`. Reuse `session_import/convert.rs`
   normalization so live and imported records are shape-identical.
3. Adopt `StoreChatHistory` as the in-run history mechanism where the shell
   currently hand-threads message vectors (evaluate: it must preserve
   native tool-call envelopes and XML/P-format compat suffixes verbatim —
   if not, keep OpenHuman assembly and only persist through the store).
4. Write-side parity test: run a turn, assert legacy JSONL and store stream
   render identical transcripts (reuse importer's JournalMessage parity
   helper).

## Deletions

None yet (dual-write phase). Deletion lands in 04.2.

## Acceptance

- Every new turn appears in the store journal with correct stem, thread_id,
  provider/model metadata, tool-call ids.
- Dual-write behind one flag so 04.2 can flip reads independently.
