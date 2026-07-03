# 05 — Events, status, observability

Make crate journals/status stores the canonical run record; everything
OpenHuman-specific becomes a projection (AgentProgress, DomainEvent, run
ledger, cost footer).

Target SDK surface: `HarnessEventJournal` (`StoreEventJournal` over
`JsonlAppendStore`), `HarnessStatusStore` (`list_by_root`, `list_by_thread`,
`list_active`), `AgentObservation { run_id, parent_run_id, root_run_id,
offset }`, `GraphEventJournal`/`GraphStatusStore`, `RedactingSink`,
`FanOutSink`, `RecordingListener`, Langfuse exporters (optional).

Steps:

1. `01-journals.md` — persist journals + status; late-attach replay.
2. `02-bridge-consolidation.md` — one bridge; delete `engine/`.
3. `03-domainevent-projection.md` — DomainEvent/AgentProgress as projections.

Done when: a UI can reconstruct a running/completed turn from persisted
crate events without subscribing at start; run-ledger event rows are fed from
journal records. `agent/harness/engine/` is already deleted.

Risk: crosses UI streaming, cost footer, desktop reconnect — parity tests
before each deletion.
