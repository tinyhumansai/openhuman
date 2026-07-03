# 04 — Sessions & stores

Make tinyagents `Store`/`AppendStore` the durable session substrate and
retire the legacy transcript stack. The Phase-1 write-only importer already
exists (`src/openhuman/session_import/`, `openhuman.session_import_run`,
design in `../../tinyagents-session-migration-design.md`).

Target SDK surface: `Store`/`FileStore`, `AppendStore`/`JsonlAppendStore`,
`StoreRegistry` on `RunContext`, `ChatHistory`/`StoreChatHistory`,
`SessionId`/`ThreadId`/`RunId`, `HarnessRunStatus`.

Steps:

1. `01-live-writes.md` — new turns write tinyagents records first.
2. `02-read-cutover.md` — import phases 2–4: shadow reads, cutover, retire.
3. `03-checkpointer.md` — SqliteCheckpointer swap (needs 00-baseline).

Done when: `session/transcript.rs`, `session/migration.rs`,
`session/turn/session_io.rs`, and `session_import/` are deleted; readers
serve from `tinyagents_store/`; run inspection uses crate lineage ids.

CONSTRAINT (from importer work): store/stream names are slash-free —
per-session streams are `session.{stem}.messages`.

Risk: transcript shape is user data — every cutover step keeps the legacy
reader until parity fixtures (11-fixture matrix in the design doc) pass.
