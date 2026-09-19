# announcements

Thin RPC adapter for the product-announcements feed. Like its `hosted`
siblings it owns no business logic, state, or authorization — it forwards an
authenticated request to the TinyHumans backend and passes the response
through verbatim.

## Responsibilities

- Fetch the latest active announcement for the signed-in user via
  `GET /announcements/latest`.
- Resolve and require a live backend session token before calling out; fail
  closed with a clear error when none is stored.
- Fold the backend's 404 (`BackendApiError::AnnouncementNotFound`, no
  qualifying announcement) into the same `null` "no announcement" success
  outcome instead of surfacing it as an error — the feature is cosmetic and
  the 404 is a normal outcome, not a failure worth reporting.

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | Re-exports `ops::*` and the schema/controller pair. |
| `ops.rs` | `require_token`, `get_latest_announcement`. Builds a `BackendOAuthClient` against the effective backend URL and issues the authed GET. |
| `schemas.rs` | Controller schema + handler that loads `Config` and delegates to `ops`. |
| `ops_tests.rs`, `schemas_tests.rs` | 404-detection and schema/registration tests. |

## RPC / controllers

One controller in the `announcements` namespace (schema `function: "get_latest"`),
registered into the global registry via `crates/openhuman-core/src/core/all.rs`:

| Wire method | Inputs | Output | Backend call |
| --- | --- | --- | --- |
| `openhuman.announcements_get_latest` | none | `announcement` (JSON, may be `null`) | `GET /announcements/latest` |

## Persistence

None. The domain reads the stored session token but does not persist
anything. Dismissal is tracked client-side by announcement id
(`app/src/store/announcementSlice.ts`, `shownIds`, persisted through
`userScopedStorage`) — this module has no notion of "dismissed".

## Dependencies

- `crate::security::credentials::session_support::require_live_session_token`
  — rejects an expired token locally instead of firing a doomed backend 401
  (same guard as `billing/ops.rs`).
- `crate::api::config::effective_backend_api_url`, `crate::api::BackendOAuthClient`
  — resolve the backend base URL and issue the authed JSON request, carrying
  the sanitized `x-sdk-name` product identity (`crate::api::product`) on every
  call.
- `crate::api::flatten_authed_error` — flattens any non-404 backend/session
  error for the RPC caller.
- `crate::rpc::RpcOutcome` (re-export of `openhuman_rpc`) — return wrapper
  carrying value + log line.

## Gating

`announcements` is part of `DomainGroup::Hosted` (`crates/openhuman-core/src/core/all.rs`).
`DomainSet::embedded` sets `hosted: false`, dropping the whole `Hosted` group
as a unit, so this controller and its siblings do not register there.
