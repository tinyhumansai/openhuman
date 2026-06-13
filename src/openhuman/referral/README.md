# referral

Thin RPC adapter domain for the referral program. It does **not** own any business logic, state, or schema of its own — it makes authenticated `reqwest` calls to the hosted backend's `/referral/*` endpoints and surfaces the raw `data` payloads to the CLI / JSON-RPC clients. It exists primarily because the desktop WebView `fetch` to the backend can fail with a generic "Load failed" (CORS / TLS / WebKit), so these ops reuse the same server-side `reqwest` path as the billing domain.

## Responsibilities

- Fetch referral stats (code, link, totals, referred-user rows) via `GET /referral/stats`.
- Claim a referral code for the current user via `POST /referral/claim`, with an optional device fingerprint for abuse signals.
- Resolve and require a backend session token before any call; fail closed with a clear error when no token is stored.
- Trim the referral `code`; trim and drop whitespace-only `deviceFingerprint` before forwarding.
- Wrap backend responses in `RpcOutcome<Value>` with grep-friendly log lines.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/referral/mod.rs` | Export-only. Re-exports `ops::*` and the schema/controller pair (`all_referral_controller_schemas`, `all_referral_registered_controllers`, `referral_schemas`). |
| `src/openhuman/referral/ops.rs` | Business logic: `require_token`, `get_stats`, `claim_referral`. Builds a `BackendOAuthClient` against the effective backend URL and issues authed JSON requests. Includes inline tests against an Axum mock backend. |
| `src/openhuman/referral/schemas.rs` | Controller schemas + `handle_*` fns that load config and delegate to `ops`. Defines `ReferralClaimParams` (camelCase deserialization) and helpers (`to_json`, `deserialize_params`, `json_output`). |

## Public surface

From `mod.rs` re-exports:

- `get_stats(config: &Config) -> Result<RpcOutcome<Value>, String>` (via `ops::*`).
- `claim_referral(config: &Config, code: &str, device_fingerprint: Option<&str>) -> Result<RpcOutcome<Value>, String>` (via `ops::*`).
- `all_referral_controller_schemas() -> Vec<ControllerSchema>`.
- `all_referral_registered_controllers() -> Vec<RegisteredController>`.
- `referral_schemas(function: &str) -> ControllerSchema`.

(`require_token` is a private helper.)

## RPC / controllers

Two controllers in the `referral` namespace, registered into the global registry via `src/core/all.rs`:

| Method | Inputs | Output | Backend call |
| --- | --- | --- | --- |
| `referral_get_stats` (`referral.get_stats`) | none | `stats` (JSON) | `GET /referral/stats` |
| `referral_claim` (`referral.claim`) | `code` (string, required), `deviceFingerprint` (string, optional) | `result` (JSON) | `POST /referral/claim` |

An unrecognized `function` name returns an `unknown` placeholder schema with an `error` output. Handlers load `Config` via `config_rpc::load_config_with_timeout()` and return CLI-compatible JSON through `RpcOutcome::into_cli_compatible_json()`.

## Persistence

None of its own. The domain is stateless — it reads the backend session token from the credentials store via `get_session_token` but does not persist anything.

## Dependencies

- `crate::api::config::effective_backend_api_url` — resolves the effective backend API base URL from `config.api_url`.
- `crate::api::jwt::get_session_token` — reads the stored backend session token.
- `crate::api::BackendOAuthClient` — issues authenticated JSON requests (`authed_json`) to the backend.
- `crate::openhuman::config::Config` — config struct passed into ops; `config::rpc::load_config_with_timeout` is used by the schema handlers.
- `crate::core::all::{ControllerFuture, RegisteredController}` and `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller registry types.
- `crate::rpc::RpcOutcome` — return wrapper carrying value + logs.
- Test-only: `crate::openhuman::credentials::{AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME}` for seeding session tokens in unit tests.

## Used by

- `src/core/all.rs` — registers `all_referral_registered_controllers()` into the controller registry (line ~213) and `all_referral_controller_schemas()` into the schema list (line ~345), exposing both methods to CLI and JSON-RPC.

## Notes / gotchas

- No `types.rs`, `store.rs`, `tools.rs`, or `bus.rs` — this is a pure RPC adapter, not a stateful domain. No agent tools, no event-bus subscribers.
- Both ops fail closed when no session token is stored, with the error `"no backend session token; run auth_store_session first"`.
- Eligibility for `claim` ("only users who have not yet subscribed") is enforced **by the backend**, not in this module — it merely forwards the request.
- Trimming/whitespace-dropping of `deviceFingerprint` happens in both `ops::claim_referral` and the schema handler `handle_referral_claim` (defensive, redundant filtering).
- The module deliberately mirrors the billing domain's server-side `reqwest` path to avoid WebView `fetch` "Load failed" failures.
