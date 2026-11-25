# FINDINGS: gemini live/sessions 400 (TAURI-RUST-13BR, #8501)

## Verdict (final, backend-confirmed)
**User billing state, not a request defect.** The 400 is the backend's balance gate:
`tinyhumansai/backend` @ 6af8f69, `src/controllers/agentIntegrations/gemini/liveSessions.ts:119-125`
throws `BadRequestError("Gemini Live requires an available balance of at least $X.XX (reserved per
open session)", {reserveUsd}, USER_INSUFFICIENT_CREDITS)` when the user's balance is below the
per-session reserve. Serialized through `errorHandler.ts` that body is
`{"success":false,"error":"Gemini Live requires an available balance of at least $1.00 (reserved per open session)","errorCode":"USER_INSUFFICIENT_CREDITS","details":{"reserveUsd":1}}`,
exactly **182 bytes with keys `details,error,errorCode,success`**, matching the Sentry shape. The
request body passes the backend's strict zod schema (`validators/agentIntegration.validator.ts:278-305`;
`functionDeclarations` max 128, `tools` max 10), which rules out the tool-declaration hypothesis below.
It is systemic because `gemini-hosted` is the default live provider and counts as configured for every
signed-in user, whatever their balance.

Why it reached Sentry: the client's budget filter keys on phrases ("insufficient balance", "add
credits", ...), and this message contains none of them; the `errorCode` was never read.

## Fix (this branch)
- `client/classify.rs`: `backend_billing_refusal(status, body)` classifies an
  `errorCode: USER_INSUFFICIENT_CREDITS` envelope (any status) and the voice-agent 403 billing
  messages (see FINDINGS-voice-signed-url.md). Those are info-logged, **not** `report_error`ed, and
  returned as typed `HostedApiError::InsufficientCredits` / `PaidPlanRequired`.
- `client/error.rs`: the two variants display in the generic `<METHOD> <path> failed (<status>): <body>`
  shape and flatten onto new `INSUFFICIENT_CREDITS:` / `PAID_PLAN_REQUIRED:` sentinels.
- `openhuman-core` observability: the sentinels (`observability_availability.rs`, re-exported through
  embed) classify as `ExpectedErrorKind::BudgetExhausted`, so a re-report at any boundary is demoted.
- `voice/live.rs`: `mint_error` maps the sentinels to live-session codes `insufficient_credits` /
  `paid_plan_required` (Gemini and ElevenLabs hosted mints).
- App: `LiveVoiceControls.tsx` `errorText` shows "Live voice needs more credits. Add credits to
  continue." / "Live voice requires a paid plan. Upgrade to continue." instead of the raw backend line;
  both keys translated in all 16 locales.
- Not done (product decision): hiding or de-defaulting the hosted live providers for users below the
  reserve. `backend_ready()` still only checks that a credential exists.

## Verification of the fix
- In-tree `cargo check`/`test` still impossible (vendor pins, see below). `cargo check -p openhuman
  --tests` gives the **same error set** with and without this change (880 vs 881 lines, identical
  after normalizing line numbers, none in touched files).
- `backend_billing_refusal`, the two error variants and `flatten_authed_error` were compiled verbatim
  in a scratch crate together with the in-repo pure tests (the real 182-byte body, both 403 messages,
  non-matches) and pass. The axum round-trip test, `mint_error` test, core classifier test and the
  Vitest case were added but not run (no compiling tree; no pnpm/node_modules on this box). All 16
  locale files were parsed with node and carry both keys exactly once.

---
Earlier analysis, kept for the record (superseded by the verdict above):

## Verdict (first/second pass)
Root cause NOT confirmed. This is not a regression: live voice (including the
`gemini-hosted` provider) first shipped in v0.64.14 on 2026-10-08, the day the
issue began, so the request has most likely been rejected since launch.
Leading hypothesis: the backend rejects the *content* of the ticket body, most
likely the agent's tool declarations (see "Revised analysis"). Only
observability was changed.

## Revised analysis (second pass)
- Release timeline: the first commit with the hosted Gemini route
  (e9aa12b395, 2026-10-07) is first contained in v0.64.14 (tagged
  2026-10-08). v0.64.15 differs from v0.64.14 only by version bumps and the
  `vendor/tinybus` pin; no voice code changed. So "began 2026-10-08" is the
  feature launch, and comparing history for a renamed field was the wrong test.
- Why it is systemic: `gemini-hosted` is the default live provider
  (`default_provider()` in `config/schema/voice_live.rs`), and it counts as
  "configured" whenever any backend credential exists.
- Call path: app `LiveVoiceControls` (session over the core's
  `/ws/live-voice`) or the `LiveVoicePanel` test button →
  `voice/live/session.rs::start` / `ops.rs::live_test_provider` →
  `providers::prepare` → `mint_gemini_ticket` → POST.
- Field names match the backend contract: every key the client sends exists,
  in the right camelCase, on `GeminiLiveConversation` in
  `vendor/tinyhumans-sdk/src/api/agent_integrations/gemini.rs`, and
  `gemini-3.8-live` is a documented model there. The route is in the SDK
  manifest synced from backend `main` (`tests/openapi_sync.rs`, 208 -> 211).
- What does not match is the content: the SDK's only test body is
  `{"mode":"conversation","model":"gemini-3.8-live","tools":[{"googleSearch":{}}]}`.
  The real client sends the full agent tool catalogue as
  `functionDeclarations` (`session.rs` sets
  `live.tools = tinyagents_live::tool_declarations(&harness, false)` before
  minting) plus a long system prompt. Nothing in the repo shows the backend
  accepting that payload. The `details` key in the 400 envelope fits a
  validation error.
- Discriminating test (not run: it calls the real backend): with a dev
  account, POST the SDK's minimal body. 200 → the client payload is the
  cause; re-add `tools`, then `systemInstruction`, etc. to find the field.
  400 → server-side.

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
  or path. No renamed/removed/mistyped field or content-type change found;
  "began 2026-10-08" is the v0.64.14 feature launch (see above).
- The failing response is the standard envelope `{success,error,errorCode,details}`
  (len 182). Its text was never in telemetry: `classify.rs` reports only the
  shape to Sentry (the 512-byte body does go into the returned error string).
- Suspects for the server to check, in order (unverified): the
  `functionDeclarations` (a tool name or schema the backend rejects, or the
  tool count/size); the size of `systemInstruction`. The first-pass guesses
  (the `gemini-3.8-live` model id, the `contextWindowCompression` /
  `realtimeInputConfig` fields) are less likely: both appear in the SDK's
  backend contract.

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
