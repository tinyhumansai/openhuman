# imessage_scanner

A macOS-only background task that reads the local Messages database
(`~/Library/Messages/chat.db`) and files each conversation day as a document in
memory under the `imessage` source. It needs no webview, DOM or browser
automation: Messages keeps everything in one SQLite file, so the scanner opens
it read-only and polls. On other platforms the module compiles to an empty
`ScannerRegistry` stub.

## How it works

`lib.rs` registers an `Arc<ScannerRegistry>` as managed state, and on macOS
`setup()` calls `ensure_scanner(app, "default")`. That spawns `run_scanner`
once; later calls are no-ops. The scanner runs whether or not iMessage is
enabled and checks the config on every tick.

```text
run_scanner (every 60 s)
   |
   | cursor = read <app data dir>/imessage-cursor-default.txt (or 0)
   v
tick::run_single_tick
   |
   | 1. fetch_gate: openhuman.config_get
   |      channels_config.imessage absent/null -> skip tick
   |      allowed_contacts (empty or "*" = all chats)
   | 2. chatdb::read_since(cursor, 2000)        new messages only
   | 3. unique (chat_identifier, local day) pairs
   | 4. for each allowed pair:
   |      chatdb::read_chat_day(chat, day, 5000) whole day, not just new rows
   |      format_transcript -> "[unix_ts] sender: text" lines
   |      ingest_group: openhuman.memory_brain_ingest
   |          { text, source: "imessage", title: "Messages", chat, day }
   v
TickOutcome { new_rowid, groups_attempted, groups_ingested, ... }
   |
   | no group failed -> write new cursor
   | any group failed -> keep old cursor, retry next tick
```

The ingest title is the word "Messages" followed by the chat identifier and
the `YYYY-MM-DD` day. Each tick rebuilds the full day for every chat that received a new message,
so a document always holds the complete day rather than a fragment. The
cursor is the highest `message.ROWID` seen and only advances when every group
in the tick ingested successfully.

Message text comes from the `text` column, or, when that is NULL (newer
macOS versions), from a heuristic decode of the binary `attributedBody`
(`extract_text_from_attributed_body`). Messages with no recoverable text are
kept as `[non-text]` so the timeline stays complete. Apple stores dates as
nanoseconds since 2001-01-01; `apple_ns_to_unix` and
`local_day_bounds_apple_ns` convert them and compute local-day boundaries.

Both RPC calls go to the embedded core over HTTP, using
`core_rpc::core_rpc_url_value` and `core_rpc::apply_auth` with one shared
`reqwest::Client` (10 s timeout). A JSON-RPC error inside a 200 response (for
example memory turned off) counts as a failed group.

## Layout

| File | What it does |
| --- | --- |
| [`mod.rs`](mod.rs) | `ScannerRegistry`, the `run_scanner` loop, cursor I/O, the config gate, transcript formatting, date helpers, and `ingest_group`. |
| [`tick.rs`](tick.rs) | `run_single_tick`, the body of one tick, behind the `TickDeps` trait so it can run against a real `chat.db` without Tauri. `HttpDeps` is the production implementation. |
| [`chatdb.rs`](chatdb.rs) | Read-only SQLite access: `read_since` (rows after a cursor) and `read_chat_day` (one chat, one day). Opens with `SQLITE_OPEN_READ_ONLY`, so it never takes a write lock that could conflict with Messages. |

## Key types and entry points

- `ScannerRegistry` ([`mod.rs`](mod.rs)): `new`, `ensure_scanner`, `shutdown`. The
  registry tracks one task; the per-account shape mirrors the old webview
  scanners.
- `tick::run_single_tick` and `tick::TickDeps` ([`tick.rs`](tick.rs)): the testable unit.
- `tick::TickOutcome` (`tick.rs`): what a tick did, including
  `skipped_unconnected` and `had_group_failure`.

## Boundaries

- Memory storage and the `openhuman.memory_brain_ingest` method belong to the
  core's memory domain.
- The iMessage channel config (`channels_config.imessage`,
  `allowed_contacts`) is defined and edited in the core config.
- Granting the app Full Disk Access, which macOS requires to read `chat.db`,
  is the user's action; the scanner just fails its read until then.

## Gotchas

- The scanner always talks to the embedded core through
  `OPENHUMAN_CORE_RPC_URL`, not the active gateway. If the embedded core is not
  running (for example a remote gateway is in use), `apply_auth` fails with
  "core RPC token is not initialized" and the tick is skipped at debug level.
- `lib.rs` stops the scanner during exit teardown (`shutdown_imessage_scanner`)
  so the task does not outlive the core.
- The allowlist match is exact (case-insensitive, trimmed) against
  `chat_identifier`; there is no partial or contact-name matching.

## Tests

[`imessage_scanner_tests.rs`](imessage_scanner_tests.rs) (declared from `mod.rs`) and [`tick_tests.rs`](tick_tests.rs)
(declared from `tick.rs`) build on macOS only. Two tests in `tick_tests.rs`
read the real `~/Library/Messages/chat.db` and are `#[ignore]`d; they need
Full Disk Access.

```bash
cargo test --manifest-path crates/openhuman-app/Cargo.toml imessage_scanner::
cargo test --manifest-path crates/openhuman-app/Cargo.toml imessage_scanner::tick_tests -- --ignored
```

## Further reading

- [`docs/gitbooks/en/developing/architecture/tauri-shell.md`](../../../../docs/gitbooks/en/developing/architecture/tauri-shell.md): the Tauri shell.
- [`crates/openhuman-app/README.md`](../../README.md): the openhuman-app crate README.
