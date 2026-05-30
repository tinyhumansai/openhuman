# voice

Speech-to-text (STT) and text-to-speech (TTS) domain. Exposes the `openhuman.voice_*` RPC namespace for transcription, synthesis, availability checks, provider configuration, agent reply-speech (with mascot lip-sync visemes), and a standalone voice **dictation server** (hotkey → record → transcribe → insert text). Routing between cloud (hosted backend proxy) and local engines (whisper.cpp / Piper) is decided by a provider factory driven by config. The low-level inference implementations themselves now live under `crate::openhuman::inference::voice` and are re-exported through this module's surface for back-compat.

## Responsibilities

- Transcribe audio (file path, raw bytes, or base64) via whisper.cpp (local) or the hosted backend STT proxy, with optional LLM cleanup and hallucination filtering.
- Synthesize speech via local Piper, the hosted ElevenLabs proxy, or third-party providers (OpenAI-compatible / Deepgram / ElevenLabs) keyed by slug.
- Resolve effective STT/TTS providers from config and construct boxed `SttProvider` / `TtsProvider` trait objects (the factory).
- Run a hotkey-driven dictation server: capture mic audio, gate on duration/silence/hallucination, transcribe, then either deliver via Socket.IO (when OpenHuman is focused) or paste into the active external app.
- Provide a core-side dictation hotkey listener that broadcasts press/release and transcription events to the Socket.IO bridge.
- Synthesize agent replies with Oculus-15 viseme alignment for mascot lip-sync.
- Persist STT/TTS provider selection and the voice provider registry into config.
- Report STT/TTS binary + model availability and the active provider selection.

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | Module docstring + exports; re-exports inference-side voice submodules (`cloud_transcribe`, `hallucination`, `local_speech`, `local_transcribe`, `postprocess`, `streaming`); defines `cloud_transcribe_default_model()` (`"whisper-v1"`). |
| `types.rs` | RPC DTOs: `VoiceSpeechResult`, `VoiceTtsResult`, `VoiceStatus` + `From<LocalAi*>` conversions. |
| `ops.rs` | Business logic returning `RpcOutcome<T>`: `voice_status`, `voice_transcribe`, `voice_transcribe_bytes`, `voice_tts`, `normalize_extension`. |
| `schemas.rs` | Controller schemas, registry exports, and all `handle_voice_*` / `handle_overlay_stt_notify` RPC handlers (param structs, provider-string validation, `voice_list_models` presets, `voice_test_provider`). |
| `factory.rs` | `SttProvider` / `TtsProvider` traits; cloud/whisper/piper/external implementations; `create_stt_provider` / `create_tts_provider`; `effective_*_provider`; slug:model parsing; `DEFAULT_WHISPER_MODEL`, `DEFAULT_PIPER_VOICE`, `WHISPER_MODEL_PRESETS`. |
| `server.rs` | The `VoiceServer` dictation runtime: hotkey event loop, recording lifecycle, duration/silence/hallucination gates, background processing, global singleton (`global_server` / `try_global_server` / `start_if_enabled` / `run_standalone`). |
| `hotkey.rs` | rdev-based global hotkey listener; `ActivationMode` (Tap/Push), `HotkeyEvent`, `HotkeyCombination`, `parse_hotkey`, `start_listener`. |
| `audio_capture.rs` | cpal mic capture → 16 kHz mono WAV bytes; `RecordingHandle`, silence-gate ring buffer, peak-RMS reporting. |
| `text_input.rs` | Clipboard-paste text insertion (`insert_text`) — writes clipboard then simulates Cmd/Ctrl+V via enigo, restoring prior clipboard. |
| `dictation_listener.rs` | Core-side dictation broadcast bus: `DictationEvent`, `publish_dictation_event` / `subscribe_dictation_events`, `publish_transcription` / `subscribe_transcription_results`, rdev listener lifecycle (`start_if_enabled` / `stop`), `normalize_hotkey_for_rdev`. |
| `reply_speech.rs` | Agent reply synthesis via backend `/openai/v1/audio/speech`; `ReplySpeechResult`, `VisemeFrame`, `AlignmentFrame`, `ReplySpeechOptions`, `synthesize_reply`, tolerant response normalization. |
| `cli.rs` | `openhuman voice` / `openhuman dictate` subcommand adapter — runs a blocking standalone dictation server (domain-owned, since it blocks forever and doesn't fit the controller registry). |
| `*_tests.rs` | Sibling test suites: `audio_capture_tests.rs`, `schemas_tests.rs`, `server_tests.rs` (wired via `#[path = ...]`); other files use inline `#[cfg(test)]`. |

## Public surface

- Types: `VoiceSpeechResult`, `VoiceStatus`, `VoiceTtsResult` (from `types`).
- Ops (`pub use ops::*`): `voice_status`, `voice_transcribe`, `voice_transcribe_bytes`, `voice_tts`.
- Factory: `create_stt_provider`, `create_tts_provider`, `default_stt_provider`, `default_tts_provider`, `effective_stt_provider`, `effective_tts_provider`, traits `SttProvider` / `TtsProvider`, `SttResult`, `ExternalSttProvider`, `ExternalTtsProvider`, constants `DEFAULT_PIPER_VOICE`, `DEFAULT_WHISPER_MODEL`, `WHISPER_MODEL_PRESETS`.
- Schemas: `all_voice_controller_schemas`, `all_voice_registered_controllers`, `voice_schemas`.
- Re-exported inference submodules: `cloud_transcribe`, `hallucination`, `local_speech`, `local_transcribe`, `postprocess`, `streaming`.
- Submodules `server`, `hotkey`, `dictation_listener`, `reply_speech`, `text_input`, `audio_capture`, `factory` are `pub`.

## RPC / controllers

Namespace `voice` (registered in `all_voice_registered_controllers`):

| RPC method | Purpose |
| --- | --- |
| `voice.status` | STT/TTS binary + model availability and active provider selection. |
| `voice.transcribe` | Transcribe a file path (whisper.cpp), optional LLM cleanup. |
| `voice.transcribe_bytes` | Transcribe raw audio bytes (writes temp file), with hallucination filter + cleanup. |
| `voice.tts` | Synthesize speech to a file via Piper. |
| `voice.reply_synthesize` | Synthesize an agent reply through the effective TTS provider; returns base64 audio + visemes. |
| `voice.cloud_transcribe` | Transcribe base64 audio via the hosted backend STT proxy (back-compat path). |
| `voice.stt_dispatch` | Factory-dispatched STT (cloud / whisper / `<slug>:<model>`); returns `{ text, provider }`. |
| `voice.tts_dispatch` | Factory-dispatched TTS (cloud / piper / `<slug>:<voice>`); returns `ReplySpeechResult`. |
| `voice.set_providers` | Persist STT/TTS provider + model/voice into `config.local_ai.*`. |
| `voice.update_provider_settings` | Persist the voice provider registry + routing strings (mirrors inference model settings). |
| `voice.list_models` | List models/voices for a provider (static presets for built-in slugs). |
| `voice.test_provider` | Test/validate a provider endpoint (silent-WAV STT, "Hello" TTS, or key-only validate). |
| `voice.server_start` / `voice.server_stop` / `voice.server_status` | Control the global dictation server. |
| `voice.overlay_stt_notify` | Bridge chat-button STT state transitions into the dictation/transcription buses. |

Provider strings follow the grammar `cloud`/`openhuman` (backend proxy), `whisper`/`piper` (local), `<slug>` or `<slug>:<model|voice>` (registry lookup in `config.voice_providers`).

## Events

Does not use the typed `DomainEvent` bus. Instead `dictation_listener` owns two process-global `tokio::sync::broadcast` channels:
- `DictationEvent` (`pressed`/`released`) — `publish_dictation_event` / `subscribe_dictation_events`.
- transcription text — `publish_transcription` / `subscribe_transcription_results`.

`src/core/socketio.rs` subscribes to both and forwards them to Socket.IO clients (so dictation hotkeys and results reach the frontend without Tauri-side shortcut registration).

## Persistence

No dedicated `store.rs`. State is persisted into the shared TOML `Config` via the config domain: `config.local_ai.{stt,tts}_provider`, `config.local_ai.{stt_model_id,tts_voice_id}`, top-level `config.{stt,tts}_provider`, and `config.voice_providers` (the registry). The dictation `VoiceServer` keeps in-memory runtime state only (state machine, transcription count, rolling recent-transcript buffer for whisper context) behind a `OnceCell` singleton.

## Dependencies

- `crate::openhuman::inference` — local AI runtime (`local::global`, `whisper_engine`, model id/path resolution) and the relocated voice inference impls (`inference::voice::{cloud_transcribe, local_speech, local_transcribe, hallucination, postprocess, streaming}`); also `inference::provider::factory::lookup_key_for_slug` for provider API keys.
- `crate::openhuman::config` — `Config`, `config::rpc::load_config_with_timeout`, voice-server / dictation config sections, and `config::schema::voice_providers` (`VoiceProviderCreds`, capability/auth/API-style enums).
- `crate::openhuman::accessibility` (macOS only) — focused-app detection (`focused_text_context_verbose`) and the Swift globe-key listener (`globe_listener_start` / `globe_listener_poll`) used in place of rdev for the Fn key.
- `crate::api` — `BackendOAuthClient`, `effective_backend_api_url`, `get_session_token` for backend-proxied reply-speech.
- `crate::core::all` (`ControllerFuture`, `RegisteredController`), `crate::core::{ControllerSchema, FieldSchema, TypeSchema}`, `crate::core::logging` (CLI run init), and `crate::rpc::RpcOutcome`.
- External crates: `cpal` + `hound` (capture/WAV), `rdev` (hotkeys), `enigo` + `arboard` (paste insertion), `reqwest` (external provider HTTP), `tokio`/`tokio-util`, `once_cell`.

## Used by

- `src/core/all.rs` — registers the voice controllers.
- `src/core/socketio.rs` — subscribes to the dictation/transcription broadcast buses; `streaming::handle_dictation_ws`.
- `src/core/jsonrpc.rs` — wiring.
- `src/openhuman/desktop_companion/pipeline.rs`, `src/openhuman/meet_agent/brain.rs`, `src/openhuman/audio_toolkit/ops.rs`, `src/openhuman/credentials/ops.rs` — call factory / TTS / transcription helpers.
- `src/openhuman/inference/local/{install_whisper,install_piper}.rs` — reference voice constants/presets.

## Notes / gotchas

- **macOS hotkey safety (#2677):** rdev's CGEventTap callback calls `TSMGetInputSourceProperty` off the main thread, which crashes with `EXC_BREAKPOINT` on macOS 26. So on macOS the core-side `dictation_listener::start_if_enabled` is a no-op and the voice server only supports the `fn` (Globe) key via the Swift globe listener; all other keys return an error. Non-macOS uses rdev for all keys.
- **Two server instances:** `global_server` registers the singleton observed by the `voice.server_*` RPCs; `run_standalone` (CLI) deliberately creates an isolated, unregistered `VoiceServer`.
- **Reply-speech approval-gate classification is "internal"** — if `reply_speech` is ever wrapped in a `Tool`, `external_effect()` MUST stay `false` so the approval gate never prompts on TTS (see file docstring + #1339/#1206).
- **Provider precedence:** `effective_*_provider` prefers the top-level `config.{stt,tts}_provider`, falls back to `config.local_ai.*_provider`, then defaults to `"cloud"`.
- **Piper-voice guard:** dispatch handlers only default to `DEFAULT_PIPER_VOICE` when the active provider is `piper`; sending a Piper voice id to a cloud/external endpoint would be invalid.
- **Dictation pipeline gates** (in `server.rs`): minimum duration → peak-RMS silence threshold → hallucination filter → empty-text — each drops the recording before delivery. A `session_generation` counter discards stale state transitions from superseded recordings.
- **Kokoro TTS is intentionally not implemented** in this cut; the doc in `factory.rs` describes how to add it as a new branch + sibling module.
