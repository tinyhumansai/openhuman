# redirect_links

Redirect-link shortener for token-heavy URLs. Long tracking URLs (e.g. `trip.com/forward/...?bizData=...`) burn model tokens every time they pass through a prompt. This domain encodes them to a short `openhuman://link/<id>` placeholder on inbound text, keeps the full URL in a local SQLite store, and expands the placeholder back to the original URL on outbound messages so the user never sees the placeholder. It also has a separate helper for tagging public `openhm.xyz` short links with a `?u=<user_id>` attribution param on the way out.

## Responsibilities

- **Shorten** a long URL to a content-addressed `openhuman://link/<id>` form and persist it (idempotent / deterministic by URL).
- **Expand** a short id back to its full URL, bumping a hit counter + `last_used_at`.
- **Inbound rewrite**: replace every long URL (≥ `min_len`, default 80) in a text blob with its placeholder, preserving surrounding prose and trailing sentence punctuation.
- **Outbound rewrite**: replace every `openhuman://link/<id>` placeholder in text back to the stored URL; unknown ids are left untouched (nothing silently disappears).
- **Public-link attribution**: append `?u=<user_id>` (URL-encoded, idempotent, fragment-safe) to `openhm.xyz/<id>` URLs, guarding against lookalike domains.
- List and remove stored links; expose all of the above over JSON-RPC.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/redirect_links/mod.rs` | Export-focused: module docstring, `mod` decls, `pub use` re-exports of ops + schemas + types; aliases `ops as rpc`. |
| `src/openhuman/redirect_links/types.rs` | Serde types: `RedirectLink`, `RewriteReplacement`, `RewriteResult`. |
| `src/openhuman/redirect_links/ops.rs` | Business logic: URL/short-URL/public-URL regexes, inbound/outbound rewrite, `append_user_id_to_public_links`, and the `rl_*` RPC handlers returning `RpcOutcome`. Holds `DEFAULT_MIN_URL_LEN = 80`. |
| `src/openhuman/redirect_links/store.rs` | SQLite persistence: `shorten`/`expand`/`peek`/`list`/`remove`, content-addressed id allocation (SHA-256 hex prefix), schema bootstrap, id<->short-URL helpers (`short_url_for`, `id_from_short`, `SHORT_URL_PREFIX`). |
| `src/openhuman/redirect_links/schemas.rs` | Controller schemas (`all_controller_schemas`, `all_registered_controllers`, `schemas`) + `handle_*` fns delegating to `ops.rs`. |

## Public surface

Re-exported from `mod.rs`:

- Functions (via `pub use ops::…`): `shorten_url`, `expand_link`, `rewrite_inbound`, `rewrite_outbound`, `rewrite_outbound_for_user`, `append_user_id_to_public_links`.
- `pub use ops as rpc` — exposes the `rl_*` handlers under `redirect_links::rpc`.
- Schemas: `all_redirect_links_controller_schemas`, `all_redirect_links_registered_controllers`, `redirect_links_schemas`.
- Types: `RedirectLink`, `RewriteReplacement`, `RewriteResult`.

Also defined (not re-exported through `mod.rs`): `ops::rewrite_inbound_with_threshold`, `ops::DEFAULT_MIN_URL_LEN`; `store::{shorten, expand, peek, list, remove, short_url_for, id_from_short, SHORT_URL_PREFIX}`.

## RPC / controllers

Namespace `redirect_links` (RPC methods `openhuman.redirect_links_<function>`):

| Function | Inputs | Output |
| --- | --- | --- |
| `shorten` | `url: String` | `link: RedirectLink` |
| `expand` | `id: String` | `link: RedirectLink` (errors if not found) |
| `list` | `limit?: u64` (default 50, max 1000) | `links: RedirectLink[]` (newest first) |
| `remove` | `id: String` | `{ id, removed: bool }` |
| `rewrite_inbound` | `text: String`, `min_len?: u64` (default 80) | `result: RewriteResult` |
| `rewrite_outbound` | `text: String` | `result: RewriteResult` |

Handlers load config via `config::rpc::load_config_with_timeout()` and return `RpcOutcome<T>` serialized with `into_cli_compatible_json()`.

## Persistence

SQLite DB at `{config.workspace_dir}/redirect_links/links.db`, table `redirect_links`:

| Column | Notes |
| --- | --- |
| `id` | TEXT PRIMARY KEY — SHA-256(url) hex prefix, 8 chars default, grown by 2 up to 32 on prefix collision with a different URL. |
| `url` | TEXT NOT NULL UNIQUE (indexed). |
| `created_at` | RFC3339 TEXT. |
| `last_used_at` | RFC3339 TEXT, nullable; set on each expand. |
| `hit_count` | INTEGER, bumped on each expand. |

Insert is atomic (`ON CONFLICT DO NOTHING`) so concurrent shortens of the same URL converge on one id with no PRIMARY KEY / UNIQUE error (regression-tested). The connection is opened per call and the schema is created if missing.

## Dependencies

- `crate::openhuman::config::Config` — supplies `workspace_dir` for the DB path; handlers call `config::rpc::load_config_with_timeout`.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`) and `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller registry wiring.
- `crate::rpc::RpcOutcome` — RPC return envelope.
- External crates: `rusqlite` (SQLite), `sha2` + `hex` (content-addressed ids), `regex` (URL matching), `chrono` (timestamps), `urlencoding` (user-id encoding), `serde`/`serde_json`, `anyhow`.

## Used by

- `src/core/all.rs` registers the controllers + schemas and maps the `"redirect_links"` namespace description; `src/openhuman/mod.rs` declares the module. No other in-tree Rust callers of the rewrite/shorten functions were found — the rewrite pipeline is currently reachable via RPC rather than wired into an inbound/outbound message path inside the core.

## Notes / gotchas

- **Ids are content-addressed, not random**: same URL → same id (deterministic, deduped). Removing a link and re-shortening the same URL yields the same id again.
- **Length threshold guards token waste**: the placeholder is ~24 bytes, so URLs below `DEFAULT_MIN_URL_LEN` (80) are left untouched by inbound rewrite.
- **Trailing punctuation handling**: inbound rewrite and public-link tagging strip trailing `. , ; : !` so prose like "see https://…/path." doesn't capture the period into the stored/tagged URL.
- **`append_user_id_to_public_links` is anchored to `openhm.xyz`** specifically and rejects lookalikes (`evil-openhm.xyz`, `openhm.xyz.evil.com`); it splits off `#fragment` so `?u=` always lands in the query, and is idempotent against existing `?u=`/`&u=`.
- **`id_from_short`** accepts both `openhuman://link/<id>` and a bare hex `<id>`, lowercasing the result; non-hex input returns `None`.
- **No agent tools, no event-bus subscribers, no `bus.rs`/`tools.rs`** — this domain is store + ops + RPC only.
