# FINDINGS: voice-agent signed-url 403 (TAURI-RUST-11G3, GitHub #8504)

## Found
- Caller: `mint_voice_agent_signed_url` in `crates/openhuman-tinyhumans/src/voice/realtime.rs`
  (used by `voice/live.rs` and the `voice/schemas.rs` RPC). It does
  `resolve_backend_credential(config)` (API key -> `x-api-key`, else session JWT -> Bearer),
  `ensure_secure_backend_url`, then `BackendClient::authed_json(GET /voice-agent/get-signed-url, no body)`.
- Auth/header handling lives in `crates/openhuman-tinyhumans/src/client/` + `transport/`. 401 is
  mapped to typed `Unauthorized`/`ApiKeyRejected` (session-expiry, not reported). 403 is
  deliberately not treated as session expiry (test `authed_json_403_is_not_demoted_to_unauthorized`),
  so it falls through to `report_error` with only `response_body_len` + `body_shape`.
- A 403 means the credential WAS accepted (a bad/expired one would be 401 and is handled
  separately) but the backend refused it for this route. The request is a bodyless GET with the
  same credential path as every other authed call, so I found no client-side malformed/missing
  field or header. The git history of `voice/realtime.rs` and `client/backend_client.rs` shows only
  refactors (credential module moves, TinySecurity migration), no change to this endpoint's auth flow.
- Telemetry only has the body shape `keys=[error,errorCode,success]`; the actual `error`/`errorCode`
  values are not available, so the reason for the 403 cannot be determined from this repo.

## Hypothesis (not fixed: no client-side defect found)
Most likely server-side entitlement/authorization in the TinyHumans backend repo (e.g. the account
or plan not permitted to use voice-agent, or the backend rejecting API-key credentials / a
particular account state for `/voice-agent/*`). Only ~17 users, consistent with a per-account
condition rather than a systemic client regression. Unconfirmed: needs the `errorCode` (now
captured, below) or backend logs.

## Changed
- `client/classify.rs`: new `backend_api_error_code(body)` extracts the backend `errorCode` only if
  it is a short ASCII identifier (reuses `is_schema_like_key`), and it is appended to the
  `report_error` message as `error_code=<code|none>` and emitted on a `tracing::warn!`. This applies
  to all reported non-2xx backend responses, so 403s become distinguishable/groupable in Sentry.
  The free-text `error` value is intentionally NOT added to the report/log: the existing
  PII guard (#4058) states raw bodies must not reach pre-scrub logs. (The returned error string
  still carries the first 512 bytes of the body to the caller, unchanged.)
- Test `backend_api_error_code_only_echoes_identifier_shaped_codes` in `client/backend_client_tests.rs`.

## Verified
- The `backend_api_error_code` logic was verified in an isolated scratch crate (asserts pass).
- `cargo test -p openhuman-tinyhumans` / `cargo check` could NOT be run: the `openhuman-core`
  (`openhuman` lib) dependency fails to compile with ~400 errors on an UNMODIFIED
  upstream/main checkout (confirmed via `git stash`). Examples: `tinychannels_bus::ContentPart`
  / `MediaReference` / `OutboundVoice` not found, `ReleaseAsset` missing `library_sha256`,
  `TurnRunnerConfig` has no `cancel_grace`, `SecretStorageError::MasterKeyUnavailable` not covered.
  All `vendor/*` submodules match the commits recorded in HEAD, so the pinned submodule revisions
  appear out of sync with core source. This is a vendor/submodule-pin issue; I did not edit vendor/.
  The edit was `cargo fmt`ed.
