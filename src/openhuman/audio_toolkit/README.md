# audio_toolkit

Text-to-speech "podcast" toolkit. Synthesizes text into a workspace audio file via the `voice` TTS providers, optionally emails that file as an attachment (over SMTP or as a workspace capture in test/e2e mode), and exposes both as agent tools and JSON-RPC controllers. The combined `generate_and_email` flow chains both steps so the agent can turn arbitrary text into a listen-later audio email in one call.

## Responsibilities

- Synthesize text → audio (`mp3`/`wav`) using a configured or requested TTS provider (`cloud` default, or `piper`).
- Resolve and harden output paths: workspace-relative only, no absolute paths, no `..` traversal; default path is `artifacts/audio/<ts>-<slug>.<ext>`.
- Enforce provider/format compatibility (`piper` → `wav` only; `cloud` → `mp3` only) and verify the provider's returned MIME matches the requested format.
- Build a multipart email (plain body + audio attachment) and deliver it via the `EmailChannel` over SMTP.
- In `e2e-test-support` builds (or when `OPENHUMAN_EMAIL_CAPTURE_DIR` is set), capture the `.eml` to a workspace file instead of sending.
- Surface all three operations as agent tools and JSON-RPC controllers.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/audio_toolkit/mod.rs` | Export-only module surface: re-exports ops fns, schema registries, and types. |
| `src/openhuman/audio_toolkit/types.rs` | Serde domain types: `AudioFormat`, request/result structs. |
| `src/openhuman/audio_toolkit/ops.rs` | Business logic: `generate_podcast`, `email_podcast`, `generate_and_email_podcast`, `resolve_email_capture_dir`, plus private helpers (path/format/voice resolution, base64 decode, MIME enforcement, slugify, email build/capture). Contains the unit-test suite. |
| `src/openhuman/audio_toolkit/schemas.rs` | RPC controller schemas + `handle_*` handlers delegating to `ops.rs`; loads `Config` via `config::rpc`. |
| `src/openhuman/audio_toolkit/tools.rs` | Re-exports the three agent tool structs from `tools/podcast.rs`. |
| `src/openhuman/audio_toolkit/tools/podcast.rs` | `Tool` impls: `AudioGeneratePodcastTool`, `AudioEmailPodcastTool`, `AudioGenerateAndEmailPodcastTool`; security gating + arg parsing. |

## Public surface

From `mod.rs`:

- **ops**: `generate_podcast`, `email_podcast`, `generate_and_email_podcast`, `resolve_email_capture_dir` — all take `&Config` and return `Result<RpcOutcome<T>, String>` (except `resolve_email_capture_dir → Option<PathBuf>`).
- **schemas**: `all_audio_toolkit_controller_schemas`, `all_audio_toolkit_registered_controllers`.
- **types**: `AudioFormat` (`Mp3`/`Wav`, with `extension()` / `mime()`), `AudioGenerateRequest`, `EmailPodcastRequest`, `AudioGeneratedArtifact`, `AudioEmailDeliveryResult`, `AudioToolkitGenerateAndEmailResult`.
- **tools** (`pub mod tools`): `AudioGeneratePodcastTool`, `AudioEmailPodcastTool`, `AudioGenerateAndEmailPodcastTool`.

## RPC / controllers

Namespace `audio_toolkit` (invoked as `openhuman.audio_toolkit_<function>`):

| Method | Inputs | Output |
| --- | --- | --- |
| `audio_toolkit_generate_podcast` | `text` (req); optional `title`, `output_path`, `provider`, `voice`, `format` | `audio` (artifact JSON) |
| `audio_toolkit_email_podcast` | `to`, `subject`, `body`, `audio_path` (req); optional `attachment_name` | `email` (delivery JSON) |
| `audio_toolkit_generate_and_email_podcast` | `text`, `to`, `subject`, `body` (req); optional `title`, `output_path`, `provider`, `voice`, `format`, `attachment_name` | `result` (combined JSON) |

Registered into the global controller registry via `src/core/all.rs` (both `all_audio_toolkit_registered_controllers` and `all_audio_toolkit_controller_schemas`).

## Agent tools

From `tools/podcast.rs` — all `PermissionLevel::Execute`, each calls `SecurityPolicy::enforce_tool_operation(ToolOperation::Act, ...)` before running:

| Tool name | Backing op |
| --- | --- |
| `audio_generate_podcast` | `generate_podcast` |
| `audio_email_podcast` | `email_podcast` |
| `audio_generate_and_email_podcast` | `generate_and_email_podcast` |

Tools are constructed with `Arc<Config>` + `Arc<SecurityPolicy>` and registered in `src/openhuman/tools/ops.rs`; re-exported via `src/openhuman/tools/mod.rs`.

## Persistence

No durable domain store. Side effects are filesystem writes within the workspace:

- Audio files → `artifacts/audio/` (or caller-supplied `output_path`).
- Captured emails (test/e2e) → `artifacts/email-capture/` (or `OPENHUMAN_EMAIL_CAPTURE_DIR`) as `.eml` files named `podcast-email-<ts>-<uuid>.eml`.

## Dependencies

- `crate::openhuman::voice` — `create_tts_provider`, `DEFAULT_PIPER_VOICE`; actual speech synthesis.
- `crate::openhuman::channels::email_channel::EmailChannel` — SMTP delivery of the built message.
- `crate::openhuman::config::Config` / `config::rpc` — workspace dir, `local_ai.tts_provider`, `channels_config.email`; `load_config_with_timeout` in RPC handlers.
- `crate::openhuman::security` — `SecurityPolicy` / `ToolOperation` to gate the agent tools.
- `crate::openhuman::tools::traits` — `Tool`, `ToolResult`, `PermissionLevel`.
- `crate::core::all` / `crate::core` — `RegisteredController`, `ControllerFuture`, `ControllerSchema`, `FieldSchema`, `TypeSchema` for RPC registration.
- `crate::rpc::RpcOutcome` — controller/op return contract.
- External crates: `lettre` (email message + attachment), `base64`, `chrono`, `uuid`.

## Used by

- `src/core/all.rs` — registers the controllers/schemas into the JSON-RPC + CLI surface.
- `src/openhuman/tools/ops.rs` — instantiates the three tools into the agent tool registry; `src/openhuman/tools/mod.rs` re-exports them.

## Notes / gotchas

- `provider`/`format` are coupled: `piper` only emits `wav`, `cloud` only emits `mp3` — mismatches are hard errors (`resolve_format`). After synthesis the returned MIME is re-checked against the requested format (`enforce_audio_format`).
- Default voice is only injected for `piper` (`DEFAULT_PIPER_VOICE`); `cloud` defaults to no explicit voice.
- `email_podcast` re-validates `audio_path` (workspace-relative, no `..`) independently of `generate_podcast`; the combined flow overwrites the email request's `audio_path` with the freshly generated file's path.
- Email capture mode is feature/env-gated: enabled under `feature = "e2e-test-support"` or when `OPENHUMAN_EMAIL_CAPTURE_DIR` is non-empty; otherwise SMTP send requires `channels_config.email` to be configured (`from_address` falls back to `openhuman@localhost.test`).
- `AudioEmailDeliveryResult.mode` is `"capture"` or `"smtp"` to indicate which path ran.
