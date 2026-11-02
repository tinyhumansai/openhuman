# FINDINGS: voice-provider error handling (Workstream C)

Branch: `fix/voice-provider-error-handling` (GitHub #8502, #8503). PR: tinyhumansai/openhuman#8521.

## Correction (second commit): hosted vs BYOK
CONTEXT.md treated `payment_required` as a user config problem. The backend source
(`tinyhumansai/backend` @ 6af8f69, read-only clone) shows that is wrong for the hosted routes:
- `src/services/elevenlabs/elevenlabsService.ts` always calls ElevenLabs with the **server's own**
  `ELEVENLABS_API_KEY` (all 5 call sites) and wraps any upstream failure as
  `BadRequestError("ElevenLabs request failed (<status>): <body>")` (line 107), i.e. a 400.
- `src/middlewares/errorHandler.ts` does not report 4xx to the backend's Sentry, so the client report is
  the only signal anyone gets.

So **TAURI-RUST-15BM (TTS) and -15CT (STT) are TinyHumans' own ElevenLabs subscription with a failed
payment**, not the users'. Telling 44 + 17 users to "update billing" is wrong, and demoting the event
hides an outage from the only people who can fix it. The first commit did both. Now:
- Hosted failures (detected by the backend's `ElevenLabs request failed (` wrapping) are never matched by
  `voice_provider_user_message` / `is_voice_provider_user_state_message`, so they stay Sentry errors.
- The `classify.rs` demotion from the first commit is reverted: `authed_json` reports them again.
- Hosted TTS (`openhuman-tinyhumans` `voice/reply_speech.rs`) uses `map_hosted_voice_error`. Only failures
  of the backend's own ElevenLabs account or service (billing codes or 402, 401/403/412, 429, 5xx, read from
  the wrapper's upstream status) become "The voice service is temporarily unavailable. Please try again
  later."; request errors the user can fix (`voice_not_found`, text too long, other 4xx) keep their message.
  The raw error was already reported by `authed_json`; the mapping logs only the category and upstream status,
  and the backend-error classifier marks the exact neutral message as not reportable again.
- Hosted STT (`cloud_transcribe.rs`) is left raw: the RPC boundary is its only report site, and mapping
  there would strip the detail ops needs.
- The BYOK handling is kept: the external STT/TTS providers use the user's own key, so for them
  `payment_required` ("Update billing") and the key-ID message (TAURI-RUST-1635) are correct, and demoted.

**The real fix for 15BM / 15CT is operational**: settle the TinyHumans ElevenLabs subscription. The
backend now surfaces upstream billing/account refusals as a 503 and alerts on them
(tinyhumansai/backend#1438).

## What I found
- The ElevenLabs HTTP clients live in the vendored `tinyinference-voice` crate (`external_stt.rs` / `external_tts.rs`),
  which formats errors as `[voice-stt] elevenlabs error {status}: {body}` / `[voice-tts] ... `. Not edited (vendor).
  The host wraps them in `crates/openhuman-core/src/voice/factory/{stt,tts}_providers.rs`.
- Hosted TTS (`POST /openai/v1/audio/speech`) goes `openhuman-tinyhumans/src/voice/reply_speech.rs` ->
  `BackendClient::authed_json` -> `client/classify.rs::finish_authed_json`. Any non-2xx that is not 401, budget, or
  transient goes to `report_error` (Sentry error) with only the body *shape*.
- Hosted STT goes `cloud_transcribe.rs` -> `tinyinference_voice::cloud::transcribe` (not `authed_json`); only 401 is
  classified there.
- Existing convention: `core/observability*.rs` `expected_error_kind` -> `ExpectedErrorKind::ProviderUserState`
  (logged at info, not sent to Sentry).

## What I changed (net)
- `crates/openhuman-core/src/core/observability_voice_provider_errors.rs` (included into `observability.rs`):
  - `voice_provider_user_message()`: BYOK `payment_required` -> "Your ElevenLabs subscription has a payment issue.
    Update billing to continue."; "API key ID used as API key" -> "That looks like an ElevenLabs key ID, not a key.
    Keys start with `sk_`. Re-enter the API key."; hosted wrapping -> `None`.
  - `is_voice_provider_user_state_message()`: raw BYOK text and the two mapped messages (RPC re-report stays demoted).
  - `map_voice_provider_error()` (BYOK). The hosted mapping lives in `openhuman-tinyhumans` (above).
- `expected_error_kind` returns `ProviderUserState` for the BYOK cases only.
- Mapping applied in `ExternalSttProvider`, `ExternalTtsProvider` (BYOK) and `reply_speech.rs` (hosted).
- Tests: `observability_error_classification_voice_provider_tests.rs` (BYOK payment, key ID, hosted TTS/STT not
  claimed, hosted neutral mapping, mapped-message re-report, pass-through, no over-match).

## TAURI-RUST-112J classification
**Backend / upstream, not a client defect, and probably not user credits.** Same route (`/openai/v1/audio/speech`),
400, envelope `error,errorCode,success` (no `details`). The backend's low-credit answer on this route is
`BadRequestError('Insufficient budget')` (`controllers/inference/textToSpeech.ts:57`), which the client's existing
budget filter already keeps out of Sentry (present in 0.64.15), so a reported 400 here is most likely another
`ElevenLabs request failed (...)` upstream failure (voice not found, quota, the same billing problem). Unknown 400s
still report. To close it: read the body from one user's daily log (the error string carries the first 512 bytes),
or check whether 112J drops once the ElevenLabs billing is fixed.

## Verification (limited, please read)
- `cargo check -p openhuman` fails on a **clean** `upstream/main` too (~400 errors): vendored submodules are pinned to
  commits older than the API the host code uses (e.g. `tinychannels_bus::module_progressive::ChannelOutputEvent`).
  This blocks in-tree `cargo check/test` for `openhuman`, `openhuman-tinyhumans` and `openhuman-embed`. vendor/ not
  touched; needs submodule pins bumped upstream.
- The corrected `observability_voice_provider_errors.rs` was compiled verbatim in a scratch crate (stubbed
  `tracing`) with the test file's real error strings: BYOK payment and key ID map, hosted TTS/STT are not
  claimed and pass through, hosted maps to the neutral message, no over-match. All pass. The parts that need
  `expected_error_kind` (the demotion asserts) and the wiring edits (`reply_speech.rs`, embed re-export,
  `classify.rs` revert) were formatted with rustfmt and checked by reading, NOT compiled in-tree.
- Note: sccache `EAGAIN` occurs with default parallelism on this box; use `cargo check -j 12`.
