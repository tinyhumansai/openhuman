# Memory tree — jobs

Async job pipeline driving extraction, scoring, summarisation, and digesting off the ingest hot path. Replaces the previous synchronous `append_leaf → cascade_seal → LLM summarise` chain with a SQLite-backed queue (`mem_tree_jobs`) and a worker pool. Producers commit side-effect + follow-up job atomically inside one transaction via `enqueue_tx`.

## Pipeline shape

```text
ingest::persist          → enqueues `extract_chunk`
worker pool (3 tasks):
  extract_chunk          → LLM extraction → admission → enqueue `append_buffer` + `topic_route`
  append_buffer          → push to L0 → enqueue `seal` if gate met
  seal                   → seal one level → enqueue parent seal if cascading
  topic_route            → match topics → enqueue per-topic `append_buffer`
  digest_daily           → call `tree_global::digest::end_of_day_digest`
  flush_stale            → enqueue seals for time-stale buffers
scheduler (1 task)       → daily wall-clock tick → `digest_daily(yesterday)` + `flush_stale(today)`
```

## Public surface

- `pub fn enqueue` / `enqueue_tx` / `claim_next` / `mark_done` / `mark_failed` / `recover_stale_locks` / `get_job` / `count_by_status` / `count_total` — `store.rs` — queue persistence.
- `pub fn start` / `wake_workers` — `worker.rs` — spawn the worker pool (idempotent) and notify idle workers.
- `pub fn trigger_digest` / `backfill_missing_digests` — `scheduler.rs` — manual digest enqueues.
- `pub fn drain_until_idle` — `testing.rs` — deterministic test runner that processes all eligible jobs.
- `pub enum JobKind` / `JobStatus` / `pub struct Job` / `NewJob` / payload structs (`ExtractChunkPayload`, `AppendBufferPayload`, `SealPayload`, `TopicRoutePayload`, `DigestDailyPayload`, `FlushStalePayload`) and `NodeRef` / `AppendTarget` — `types.rs`.
- `pub const DEFAULT_LOCK_DURATION_MS` — `store.rs` — claim lease window (5 min).

## Files

- `mod.rs` — module surface and re-exports.
- `types.rs` — `JobKind`, `JobStatus`, payload structs, `NewJob` builders. Each payload owns its `dedupe_key()` so duplicates in flight are silently suppressed.
- `store.rs` — SQLite persistence: `INSERT OR IGNORE` + partial unique index on `dedupe_key WHERE status IN ('ready','running')` for at-most-one-active dedupe; `claim_next` is a single `UPDATE ... RETURNING`; `mark_done`/`mark_failed` are claim-token gated to make stale-worker settlements no-ops.
- `worker.rs` — three worker tasks plus startup `recover_stale_locks` and a 3-permit semaphore around LLM-bound jobs. Calls into `crate::openhuman::scheduler_gate::wait_for_capacity()` before claiming so Throttled / Paused modes back off without holding DB leases.
- `scheduler.rs` — daily tick at UTC 00:05 that enqueues `digest_daily(yesterday)` + `flush_stale(today)`; `trigger_digest` and `backfill_missing_digests` are manual catch-up helpers.
- `handlers/` — per-`JobKind` handler implementations.
- `testing.rs` — `drain_until_idle` for tests that need the pipeline to settle synchronously.
