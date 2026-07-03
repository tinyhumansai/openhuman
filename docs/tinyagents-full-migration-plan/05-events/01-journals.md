# 05.1 — Durable journals + status stores

## Steps

1. In `assemble_turn_harness`, attach a `StoreEventJournal` (over the
   04-sessions `JsonlAppendStore`) via a `JournalSink` on the run's
   `EventSink`, plus a `HarnessStatusStore` writer (start with the
   in-memory impl process-wide; durable Store-backed status is the target —
   write a small `Store`-backed `HarnessStatusStore` impl if the crate has
   none).
2. Same for graphs: `JournalGraphSink` + `GraphStatusStore` on delegation/
   workflow/teams/fanout runs.
3. Wrap UI-bound sinks with `RedactingSink`; define the redaction policy
   (prompts, tool args/results, secrets) in one OpenHuman module.
4. Late-attach replay: an RPC (`agent.run_events`?) that reads
   `read_from(run_id, offset)` + status so the desktop can reconnect
   mid-run and backfill — replaces reliance on transient `AgentProgress`
   buffering.
5. Feed `session_db/run_ledger` event/telemetry rows FROM journal records
   (projection job or write-through sink) instead of parallel publishes.

## Deletions

None directly; enables 05.2/05.3 deletions.

## Acceptance

- Kill the UI mid-turn, reattach, reconstruct the full timeline from
  journal + status (e2e).
- `list_by_root` answers "every active descendant of this root run".
