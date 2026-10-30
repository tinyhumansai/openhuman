# FINDINGS: voice-provider error handling (Workstream C)

Branch: `fix/voice-provider-error-handling` (GitHub #8502, #8503). Not pushed.

## What I found
- The ElevenLabs HTTP clients live in the vendored `tinyinference-voice` crate (`external_stt.rs` / `external_tts.rs`),
  which formats errors as `[voice-stt] elevenlabs error {status}: {body}` / `[voice-tts] ... `. Not edited (vendor).
  The host wraps them in `crates/openhuman-core/src/voice/factory/{stt,tts}_providers.rs`.
- Hosted TTS (`POST /openai/v1/audio/speech`) goes `openhuman-tinyhumans/src/voice/reply_speech.rs` ->
  `BackendClient::authed_json` -> `client/classify.rs::finish_authed_json`. Any non-2xx that is not 401, budget, or
  transient goes to `report_error` (Sentry error) with only the body *shape*. That is the TAURI-RUST-15BM/-112J flood.
  The returned error string still carries up to 512 bytes of the body (RPC error / daily logs).
- Existing convention: `core/observability*.rs` `expected_error_kind` -> `ExpectedErrorKind::ProviderUserState`
  (logged at info, not sent to Sentry); callers use `report_error_or_expected`.

## What I changed
- New `crates/openhuman-core/src/core/observability_voice_provider_errors.rs` (included into `observability.rs`):
  - `voice_provider_user_message()` maps `elevenlabs` + `payment_required`/"failed or incomplete payment" to
    "Your ElevenLabs subscription has a payment issue. Update billing to continue." and "API key ID used as API key"
    to "That looks like an ElevenLabs key ID, not a key. Keys start with `sk_`. Re-enter the API key."
  - `is_voice_provider_user_state_message()` matches the raw upstream text AND the mapped messages, so the re-report at
    the RPC boundary stays demoted.
  - `map_voice_provider_error()` swaps in the user message (raw error kept in a `warn` log); unknown errors pass through.
- `expected_error_kind` returns `ProviderUserState` for these (early arm, so broader 400/401 matchers cannot claim them).
- `openhuman-tinyhumans/src/client/classify.rs`: a 400 whose body matches is logged at `warn` and NOT `report_error`ed
  (same pattern as the existing budget-exhausted 400 branch).
- User-facing mapping applied in `ExternalSttProvider`, `ExternalTtsProvider` (core factory) and hosted
  `reply_speech.rs`. `openhuman-embed/src/host_internals.rs` re-exports the two helpers.
- Tests: `observability_error_classification_voice_provider_tests.rs` (payment, key-ID, mapped-message re-report,
  pass-through of unknown bodies, no over-match).

## TAURI-RUST-112J classification
Classified as **config / upstream (probable), not confirmed**; no code defect identified. Reasoning: same endpoint
(`POST /openai/v1/audio/speech`), same status (400), and a backend error envelope (`error,errorCode,success`) that
wraps an upstream provider failure, the same shape class as 15BM. But the telemetry holds only the key names, so
I cannot prove the wrapped text is `payment_required`. I deliberately did NOT demote unknown 400s on that path: if the
body is not one of the two known ElevenLabs conditions it still reports. If the 53 users' bodies are the payment
case, they are now demoted. To close it out: pull the body from a user's daily log (the error string carries the
first 512 bytes) and add its marker to `voice_provider_user_message`, or if it is a different upstream
condition, treat it as a server-side/config issue in the TinyHumans backend (different repo).

## Verification (limited, please read)
- `cargo check -p openhuman` fails on a **clean** `upstream/main` too (401 errors, identical count before/after my
  change): vendored submodules are pinned to commits older than the API the host code uses
  (e.g. `tinychannels_bus::module_progressive::ChannelOutputEvent`, `tinyflows_bus::schedule`,
  `tinyagents_harness::progress` missing). This blocks `cargo check/test` for `openhuman`, `openhuman-tinyhumans` and
  `openhuman-embed`. I did not touch vendor/. Needs submodule pins bumped upstream.
- Instead I compiled and tested the new pure module standalone (stubbed `tracing`): mapping, mapped-message
  re-match, and non-match cases pass. The wiring edits (classify.rs, reply_speech.rs, factory providers, embed
  re-export, observability include and arm) are small, were formatted with rustfmt, and checked by reading, but
  were NOT compiled in-tree; the new in-tree test file has not been run.
- Note: a sccache `EAGAIN` failure occurs with default parallelism on this box; use `cargo check -j 12`.
