# webview_accounts

Core-side support for the third-party accounts the Tauri shell hosts in CEF webviews (Gmail, WhatsApp, Telegram, Slack, Discord, LinkedIn, Zoom, Google Messages, WeChat). The core runs in-process but has **no direct CEF handle**, so this module works entirely off out-of-band data the shell hands it. It does two unrelated things: (1) a read-only probe of the shared Chromium cookie store to heuristically decide which providers have a live login, and (2) a pure normalization contract that turns scraped WeChat Web DOM data into context/memory payloads. The module has no domain state, no event subscribers, and no RPC controller of its own — it is a pure library of helper functions exported for other layers to call.

## Responsibilities

- Detect which supported webview providers currently have an active login, by inspecting Chromium's shared `Cookies` SQLite DB read-only for known per-provider session-cookie names.
- Never fail the detection probe: a missing env var, locked/corrupt/missing DB, or schema drift all map to "every provider logged_out". The result always contains a key for every tracked provider.
- Normalize WeChat Web scan payloads (chat list rows + per-peer message rows) into:
  - a context "ingest envelope"/list payload (`list_ingest_envelope` / `list_ingest_payload`), and
  - memory-document parameter maps (`memory_doc_ingest_list_snapshot`, `memory_doc_ingest_peer_transcript`) ready to be stored as memory docs.
- Validate WeChat scan payloads (`validate_scan`).

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/webview_accounts/mod.rs` | Module docstring (explains the cookie-store heuristic) + `mod`/`pub mod` decls + `pub use` re-exports. No logic. |
| `src/openhuman/webview_accounts/ops.rs` | Cookie-store probe: `Provider` table, `detect_webview_logins()`, SQLite `file:` URI construction, LIKE-escaping, plus inline unit tests. |
| `src/openhuman/webview_accounts/wechat_ingest.rs` | WeChat Web ingest contract: payload types, validation, context envelope/payload builders, memory-doc param builders, and self-contained date/time helpers. Includes inline tests. |
| `src/openhuman/webview_accounts/wechat_ingest_tests.rs` | Sibling test suite for the WeChat ingest contract (wired via `#[path = "wechat_ingest_tests.rs"] mod tests;`). |

## Public surface

Re-exported from `mod.rs`:

- `detect_webview_logins() -> serde_json::Value` — JSON object keyed by provider slug (`gmail`, `whatsapp`, `wechat`, `telegram`, `slack`, `discord`, `linkedin`, `zoom`, `google_messages`), each value a `bool`. All keys always present.
- `validate_scan(&WechatScanPayload) -> Result<(), String>`
- `list_ingest_envelope(account_id, &WechatScanPayload, ts_millis) -> Value`
- `list_ingest_payload(&WechatScanPayload) -> Value`
- `memory_doc_ingest_list_snapshot(&WechatScanPayload) -> Result<Map<String, Value>, String>`
- `memory_doc_ingest_peer_transcript(account_id, chat_id, chat_name, &[WechatMessageRow]) -> Result<Map<String, Value>, String>`
- Types: `WechatChatRow`, `WechatMessageRow`, `WechatScanPayload` (all `Serialize`/`Deserialize`).

## RPC / controllers

None. There is no `schemas.rs`, no `all_*_controller_schemas` pair, and no `handle_*` fns — this module exposes no JSON-RPC surface itself. Its functions are plain helpers intended to be called by other layers (e.g. snapshot assembly / context / memory ingest).

## Agent tools

None. No `tools.rs`.

## Events

None. No `bus.rs`; the module publishes/subscribes to no `DomainEvent`s.

## Persistence

No state of its own. It performs a **read-only** open of the shared Chromium cookie store (an external SQLite DB owned by CEF), located via the `OPENHUMAN_CEF_COOKIES_DB` env var. Opens use `file:...?mode=ro&immutable=1&nolock=1` so it can read while CEF holds an exclusive lock; stale reads are acceptable for the heuristic. The WeChat ingest helpers only produce in-memory parameter maps — they do not write anything.

## Configuration

- `OPENHUMAN_CEF_COOKIES_DB` (constant `COOKIES_DB_ENV` in `ops.rs`) — absolute path to the shared CEF `Cookies` SQLite file, exported by the Tauri shell before launching the core. Unset/empty ⇒ all providers reported logged_out. The module deliberately does **not** guess a platform default; the shell is the authoritative source of the bundle/cache path.

## Dependencies

No `use crate::openhuman::` or `use crate::core::` imports — the module depends on **no other OpenHuman domains or core modules**. External crate dependencies only:

- `rusqlite` — read-only probe of the Chromium cookie DB.
- `serde` / `serde_json` — WeChat payload (de)serialization and JSON output for `detect_webview_logins`.
- `urlencoding` — percent-encode the cookie DB path into a SQLite `file:` URI.
- `tempfile` (dev) — test fixtures.

## Used by

Declared at `src/openhuman/mod.rs:108` (`pub mod webview_accounts;`). No current in-tree Rust callers of its exported functions were found (`detect_webview_logins` and the WeChat ingest helpers have no callers under `src/` outside this module today; the only other match is an unrelated test-pattern comment in `src/openhuman/connectivity/rpc.rs`). The exports are public API kept ready for snapshot/context/memory consumers and align with the shell-side `app/src-tauri/src/webview_accounts/` provider list.

## Notes / gotchas

- **Heuristic, not authoritative.** Login detection keys off the *presence* of a known session-cookie name under a host suffix. Chromium prunes expired cookies at startup, so presence is a strong-but-not-certain signal. Session-cookie names are chosen per provider to avoid false positives from analytics/consent cookies (e.g. `NID`/`CONSENT` on google.com do not count).
- **Host matching** uses SQL `LIKE '%suffix'` with `ESCAPE '\'`; `host_suffix` values are run through `escape_like` so `_`/`%`/`\` in a future provider entry can't silently widen the match.
- **Path encoding matters.** The SQLite `file:` URI path is percent-encoded (`sqlite_uri_path`) so spaces (`/Users/John Doe/...`), `?`, `#`, `%`, and Windows `\` separators don't break URI parsing — without this, macOS users with a space in their username would silently report all-false.
- **No PII in logs.** The DB path is never logged (it can contain a username); only the env-key name is logged at `debug`.
- **`detect_webview_logins` never returns an error** — every failure path returns the all-false object so snapshot assembly always succeeds.
- **WeChat date/time helpers are hand-rolled** (`ts_to_ymd`, `format_message_stamp`, `chrono_day_key`) using a civil-date algorithm rather than pulling `chrono`, and emit UTC (`Z`) stamps.
- **Two unrelated concerns share one module** (cookie probe vs. WeChat ingest contract); they have no code dependency on each other beyond living under the same webview-accounts umbrella.
