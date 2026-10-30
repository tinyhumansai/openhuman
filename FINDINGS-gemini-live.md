# FINDINGS: gemini live/sessions 400 (TAURI-RUST-13BR, #8501)

## Verdict
Root cause NOT determinable from client code or git history. No client-side
regression found; the request body is unchanged since the feature landed.
Hypothesis: server-side validation rejection (different repo). Only
observability was changed.

## Found
- Client: `crates/openhuman-tinyhumans/src/voice/live.rs::mint_gemini_ticket`
  (v0.64.15: `crates/openhuman-core/src/voice/live/providers.rs`, identical logic)
  does `BackendClient::authed_json(cred, POST, "/agent-integrations/gemini/live/sessions", Some(body))`.
- Auth: session JWT as `Authorization: Bearer`, or API key as `x-api-key`
  (transport layer, JSON content-type via `send_json`). No query params.
- Body: `tinyliveagents::gemini::ticket_request(live)`
  (`vendor/tinyagents/vendor/tinyliveagents/.../gemini/setup.rs`), a flat object:
  `mode:"conversation"`, `model` (user setting or default `gemini-3.8-live`),
  `responseModalities:["AUDIO"]`, optional `speechConfig` (voice/languageCode),
  `systemInstruction.parts[].text`, optional `tools`, `realtimeInputConfig`,
  `contextWindowCompression:{slidingWindow:{}}`, optional `maxMinutes`.
- History: the endpoint/feature was added 2026-10-06/07 (e2609bdc64, 6e81614943
  etc.), all before v0.64.15 (2026-10-09). The vendored tinyliveagents `gemini/`
  sources are byte-identical between the v0.64.15 pin (91664e7) and the current
  checkout (e10e6eb). The move to `openhuman-tinyhumans` did not change the body
  or path. No renamed/removed/mistyped field or content-type change found, so
  "began 2026-10-08" most likely coincides with release/rollout, not a client diff.
- The failing response is the standard envelope `{success,error,errorCode,details}`
  (len 182). Its text was never in telemetry: `classify.rs` reports only the
  shape to Sentry (the 512-byte body does go into the returned error string).
- Suspects for the server to check (unverified guesses, not evidence): the
  `model` id `gemini-3.8-live` default / user-set model not accepted; the
  `contextWindowCompression`/`realtimeInputConfig` passthrough fields not in the
  backend's schema; unknown top-level keys under strict validation; payload size
  of `systemInstruction`.

## Changed
`crates/openhuman-tinyhumans/src/client/classify.rs` (+ tests in
`backend_client_tests.rs`), applies to all authed_json non-2xx reports:
- Sentry message now appends `error_code=<errorCode>` only when it is a short
  ASCII identifier (<=40 chars, same filter as safe key names).
- New opt-in gate `OPENHUMAN_LOG_BACKEND_ERROR_TEXT=1|true|yes|on`: logs the
  backend `error` and `details` text (each truncated to 300 bytes) at
  `tracing::debug!`. Off by default; never added to the Sentry report, since
  values are server-controlled free text.

## Verified
- `cargo check -p openhuman-tinyhumans` could NOT be completed: the base
  (upstream/main 218d35f4b9, verified identical failure with my change stashed)
  does not compile, with 401 errors in `openhuman-core`, e.g. unresolved
  `tinychannels_bus::module_progressive::ChannelOutputEvent`, `tinychannels_bus::web`,
  `ContentPart`. The `vendor/tinychannels` pin (f83b2a80) appears out of sync with
  core. That is a vendor/submodule issue and was not edited. Needs a
  tinychannels pin bump (or core fix) upstream.
- Instead, the new pure functions and their two unit tests were extracted into a
  scratch crate (serde_json only) and pass (2/2). `cargo fmt` was run. The
  in-repo test module was not run for the reason above.
