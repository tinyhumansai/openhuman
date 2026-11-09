# FINDINGS: voice-agent signed-url 403 (TAURI-RUST-11G3, GitHub #8504)

## Verdict (final, backend-confirmed)
**User billing state.** `tinyhumansai/backend` @ 6af8f69, `src/controllers/voiceAgent/getSignedUrl.ts:48-56`
answers `ForbiddenError` (`errorCode: FORBIDDEN`, no `details`, matching the Sentry shape
`keys=[error,errorCode,success]`) with either:
- "Voice sessions require a paid plan." (free plan), or
- "Insufficient balance to start a voice session. Add credits to continue." (balance below
  `VOICE_AGENT_MIN_BALANCE_USD`, default $1).

The client's budget filter only runs on status 400, so these 403s went to Sentry, and the UI showed the
raw backend line. The second-pass hypothesis below (entitlement exposed by the new
`elevenlabs-hosted` picker) was right in kind; the backend source settles which gate it is.

## Fix
Shared with TAURI-RUST-13BR, see FINDINGS-gemini-live.md "Fix": a 403 whose `error` names a paid plan
becomes `HostedApiError::PaidPlanRequired`; one matching the budget phrases (here "insufficient
balance" / "add credits") becomes `InsufficientCredits`. Both are kept out of Sentry and shown in the
live-voice UI as "requires a paid plan" / "add credits". A 403 with any other message (e.g. a plain
"Forbidden") still reports. The `voice.agent_signed_url` RPC path gets the same sentinel and the same
`BudgetExhausted` demotion.

---
Earlier analysis, kept for the record:

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

## Revised analysis (second pass)
- The endpoint is old: it first shipped in v0.64.0 (94ffd992d3, 2026-08-05, #5407), and its auth
  path in v0.64.15 (`crates/openhuman-core/src/voice/realtime.rs`) is unchanged.
- What v0.64.14 (2026-10-08) changed is reach: it added a second caller. Before, only the
  `voice.agent_signed_url` RPC called the endpoint; now the new `elevenlabs-hosted` live-voice
  provider (`prepare_elevenlabs`) does too.
- That provider is gated only by `backend_ready()`, which checks that a backend credential
  exists (`resolve_backend_credential(..).is_ok()`), with no plan or entitlement check. So every
  signed-in user is offered hosted ElevenLabs in the live-voice picker.

## Hypothesis (not fixed: no client-side request defect found)
The backend refuses users who are not entitled to the hosted ElevenLabs agent (plan or account
state), and v0.64.14 exposed the endpoint to them through the new picker. That fits the small
count (~17). If so, the server is right to refuse; the client-side defect is offering the provider
without an entitlement check and surfacing a generic failure. Unconfirmed. To confirm: the
backend's `error` text for one of these 403s (the `errorCode` is now captured, below), and whether
TAURI-RUST-11G3 appears on v0.64.13 or earlier in Sentry.

## Changed
Merged into `fix/gemini-live-400-and-voice-403` with workstream A. Workstream A's
`backend_api_error_code` (same behaviour) was kept; this workstream adds the `tracing::warn!`
line and an extra test. The original standalone change was:
- `client/classify.rs`: new `backend_api_error_code(body)` extracts the backend `errorCode` only if
  it is a short ASCII identifier (reuses `is_schema_like_key`), and it is appended to the
  `report_error` message as `error_code=<code|none>` and emitted on a `tracing::warn!`. This applies
  to all reported non-2xx backend responses, so 403s become distinguishable/groupable in Sentry.
  The free-text `error` value is intentionally NOT added to the report/log: the existing
  PII guard (#4058) states raw bodies must not reach pre-scrub logs. (The returned error string
  still carries the first 512 bytes of the body to the caller, unchanged.)
- Test `backend_api_error_code_only_echoes_identifier_shaped_codes` in `client/backend_client_tests.rs`
  (kept on the merged branch as `backend_api_error_code_rejects_non_identifier_and_non_string_codes`).

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
