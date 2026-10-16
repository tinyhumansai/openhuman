# voice

The voice domain is everything in the core that turns speech into text or text
into speech. It owns the `voice` RPC namespace (transcription, synthesis,
provider settings, availability), the hotkey dictation server that records,
transcribes and pastes text into the focused app, opt-in always-on listening,
agent reply speech with mascot lip-sync visemes, and live two-way voice
sessions with the agent. The frontend, the Tauri shell, channels and web chat
all call into it.

Most of the audio and transcript mechanics are not implemented here. Hosted STT
transport, Piper execution, transcript cleanup and streaming PCM handling come
from `tinyinference-voice`; VAD, resampling, WAV framing, wake-word detection,
intent routing and the hallucination filter run in the `tinyvoice` native
module; microphone capture and the hotkey listener come from the `tinyvoice`
library. This folder binds those pieces to OpenHuman config, credentials,
providers, RPC and UI contracts.

There is no local STT engine. The bundled whisper.cpp engine is retired
(`config::migrations::retire_local_whisper_stt`), so every transcription goes
either to the hosted backend proxy or to a third-party API from the voice
provider registry. TTS still has a local option (Piper) beside the hosted proxy
and third-party APIs.

## How it works

### Provider routing

Every STT and TTS request is resolved to a provider string, and the factory in
[`factory/`](./factory/) turns that string into a boxed `SttProvider` or `TtsProvider`. The
grammar is small:

| String | STT | TTS |
| --- | --- | --- |
| `cloud`, `openhuman` | hosted backend proxy | hosted backend proxy (ElevenLabs, with visemes) |
| `backend` | hosted backend proxy | not accepted (falls through to slug lookup) |
| `piper` | not an STT provider | local Piper subprocess |
| `<slug>` or `<slug>:<model or voice>` | entry in `config.voice_providers` | entry in `config.voice_providers` |

`"whisper"` and `"local"` used to select the bundled engine. They now fall
through to the slug lookup and fail by name rather than silently degrading;
`config::migrations` rewrites persisted configs so a user never reaches that
error.

Which string applies is decided by `effective_stt_provider` and
`effective_tts_provider` in [`factory/helpers.rs`](./factory/helpers.rs). TTS walks the top-level
`config.tts_provider`, then `config.local_ai.tts_provider`, then defaults to
`"cloud"`. STT walks `config.stt_provider` and `config.local_ai.stt_provider`
but accepts a value only if it names a specific provider. An empty value or
`cloud` / `openhuman` / `backend` defers to
`config.voice_server.stt_engine.provider_string()`, so an engine picked in
Settings is not shadowed by a legacy `"cloud"` default.

```text
 caller (RPC, dictation, channels, local-AI service)
        |
        v
 effective_stt_provider(config) / effective_tts_provider(config)
        |
        v
 create_stt_provider / create_tts_provider   (factory/entry.rs)
        |
   +----+------------------+-----------------------------+
   v                       v                             v
 CloudStt/TtsProvider   PiperTtsProvider          ExternalStt/TtsProvider
 (backend proxy,        (local_speech.rs ->       (slug in voice_providers,
  cloud_transcribe.rs,   tinyinference-voice)      reqwest to 3rd party)
  reply_speech.rs)
```

The local-AI service's STT path in
`inference/host_runtime/service/speech.rs` resolves its provider the same way,
which is why `voice_transcribe` in [`ops.rs`](./ops.rs) goes through
`local_ai::service::transcribe_with_prompt` rather than calling a provider
directly.

### File and byte transcription

`voice.transcribe` and `voice.transcribe_bytes` (both in [`ops.rs`](./ops.rs)) run the same
sequence. The bytes variant first writes the audio to a temp file under
`openhuman_voice_input` (extension checked by `normalize_extension`) and removes
it afterwards. The audio then goes through the effective STT provider. The
bytes path runs the raw text through the `tinyvoice` hallucination filter in
`HallucinationMode::Conversation` (if the filter is unavailable the text passes
through). Unless `skip_cleanup` is set, `postprocess::cleanup_transcription`
asks the local LLM to tidy the transcript with a three-second budget. The
result is a `VoiceSpeechResult` carrying both the cleaned `text` and the
`raw_text`.

### Hotkey dictation

The dictation server lives in [`server/`](./server/) (see [server/README.md](server/README.md)).
A hotkey press starts a recording through [`audio_capture.rs`](./audio_capture.rs); release (or a
second tap, depending on `ActivationMode`) stops it and hands the WAV to a
background pipeline:

```text
 hotkey press -> capture frontmost app name -> start_recording()
 hotkey release -> RecordingHandle::stop() -> 16 kHz mono WAV + peak RMS
        |
        v
 gate 1: duration >= min_duration_secs        else drop
 gate 2: peak_rms >= silence_threshold        else drop
        |
        v
 STT (effective provider, initial prompt from recent transcripts)
        |
        v
 gate 3: is_hallucinated(Dictation) == false  else drop
 gate 4: text not empty                       else drop
        |
        v
 focused app is OpenHuman? --yes--> publish_transcription -> Socket.IO
        | no
        v
 TinyComputer AccessibilityPaste (confidential bus call)
```

A `session_generation` counter makes each recording's state updates
conditional, so a superseded recording cannot flip the server back to `Idle`
under a newer one. `global_server` registers the singleton that the
`voice.server_*` RPCs observe; `run_standalone` (used by the CLI) builds an
isolated, unregistered `VoiceServer` on purpose.

### Always-on listening

[`always_on/`](./always_on/) keeps the microphone open and uses VAD to cut the stream into
utterances, so no hotkey is needed (see [always_on/README.md](always_on/README.md)).
It is opt-in through `config.voice_server.always_on_enabled` and pauses while
the screen is locked (macOS only; other platforms have no lock signal yet).
Each utterance is transcribed through the factory, checked against the wake
word, and either handled by a local intent fast path or published to the agent
through `dictation_listener::publish_transcription`.

### Broadcast channels and events

[`dictation_listener.rs`](./dictation_listener.rs) owns two process-global `tokio::sync::broadcast`
channels: `DictationEvent` (`pressed` / `released`) through
`publish_dictation_event` / `subscribe_dictation_events`, and transcript text
through `publish_transcription` / `subscribe_transcription_results`. The
Socket.IO server in `crates/openhuman-rpc/src/server/socketio.rs` subscribes to
both and forwards them to clients, which is how hotkeys and results reach the
frontend without Tauri-side shortcut registration. The same file starts and
stops TinyVoice's native listener (`start_if_enabled` / `stop`) and normalizes
product hotkey strings before passing them to the module.

Typed events go through the process-wide `BUS`. [`bus.rs`](./bus.rs) publishes
`DomainEvent::Voice(VoiceEvent::PttTranscriptCommitted)` with the thread id,
session id, text length, held milliseconds and a watchdog flag. The raw
transcript is never included.

### Reply speech

`reply_speech::synthesize_reply` posts the agent's reply text to the backend's
`/openai/v1/audio/speech` (ElevenLabs behind it) and returns base64 audio plus
an Oculus-15 viseme timeline the mascot uses for lip-sync. The response types
(`ReplySpeech`, `VisemeFrame`, `AlignmentFrame`) and tolerant response
normalization live in `tinyinference_voice::reply`. `voice.tts_dispatch` and
`voice.reply_synthesize` go through the factory, so a Piper or third-party TTS
provider returns the same shape (Piper with a synthetic viseme timeline).

### Live voice agents

[`live/`](./live/) runs realtime, two-way speech sessions with the agent, including tool
calls. One WebSocket at `/ws/live-voice` is one session. [`live/providers.rs`](./live/providers.rs)
knows four providers: `gemini-hosted` (a backend-minted Gemini Live relay
ticket), `elevenlabs-hosted` (a backend-signed agent URL from [`realtime.rs`](./realtime.rs)),
`gemini` (BYOK with `provider:google`) and `sarvam` (BYOK with
`provider:sarvam`). [`live/session.rs`](./live/session.rs) builds the orchestrator's session host
for the thread, wraps its tools in `agent::tinyagents::live_harness` (approval,
tool policy, CLI/RPC-only filtering, credential scrubbing) and starts a
`tinyagents_live::LiveAgent` inside a `voice` external-channel origin and
approval chat scope. [`live/persist.rs`](./live/persist.rs) writes final transcripts back into the
thread.

```text
 browser --PCM16 + JSON--> /ws/live-voice (openhuman-rpc, bearer + origin)
                                |
                                v
                     live::ws::handle_live_voice_ws
                                |
             providers::prepare (ticket / signed URL / BYOK key)
                                |
                                v
            session: LiveAgent(provider, live_harness tools)
                |                                   |
     agent speech PCM16 + events             TranscriptPersister
                |                                   |
                v                                   v
             browser                       thread messages (source=voice)
```

The wire protocol is:

- Client to core, JSON: `{"type":"start","provider"?,"thread_id"?,"client_id"?,"input_sample_rate":16000}`
  as the first frame, then `{"type":"text","text"}`, `{"type":"interrupt"}`,
  `{"type":"stop"}`. Binary frames are microphone PCM16LE mono at
  `input_sample_rate`.
- Core to client, JSON: `ready` (`session_id`, `provider`, `output_sample_rate`,
  `thread_id`), `transcript` (`role`, `text` for the utterance so far, `final`),
  `tool_started`, `tool_finished` (`ok`, `cancelled`), `interrupted`,
  `turn_complete`, `error` (`code`, `message`, `fatal`), `closed`. Binary frames
  are agent speech PCM16LE at `output_sample_rate`.

Final transcripts and typed `text` are appended to the thread as messages with
ids `voice-<session>-<n>-<role>` and `extra_metadata.source = "voice"`; the UI
reloads the thread instead of appending. A session without a thread gets a new
"Voice conversation" thread. Provider wire protocols live in `tinyliveagents`
and tool execution in `tinyagents-live`; nothing in this folder speaks a
provider's wire format.

### Realtime harness turns

[`realtime_harness/`](./realtime_harness/) is the older ElevenLabs Agents path. The backend relays each
turn of a hosted ElevenLabs session down the socket as `voice:harness`, and
`handle_voice_harness_turn` (spawned from
`platform/socket/event_handlers.rs`) runs the local orchestrator agent, the same
one chat and the meet bot use, then streams the reply back as
`voice:harness:delta`, `voice:harness:done` or `voice:harness:error`.

### Streaming dictation WebSocket

[`streaming.rs`](./streaming.rs) handles `/ws/dictation` (mounted and authenticated by
`crates/openhuman-rpc/src/server/http/dictation.rs`). The client sends PCM16
16 kHz mono binary frames and a `{"type":"stop"}` text frame; the core
accumulates the audio, encodes WAV through the `tinyvoice` module, and returns
`{"type":"final","text","raw_text"}` or `{"type":"error"}`. The `partial` frame
type is still in the protocol but is never sent, because a hosted round trip
per tick would multiply request count for text the client discards.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](./mod.rs) | Feature gate and exports. Re-exports the factory, ops, schemas and types, and defines `cloud_transcribe_default_model()` (`"whisper-v1"`). |
| [`types.rs`](./types.rs) | RPC result types `VoiceSpeechResult`, `VoiceTtsResult`, `VoiceStatus`, with `From` conversions from the local-AI types. |
| `ops.rs` | `voice_status`, `voice_transcribe`, `voice_transcribe_bytes`, `voice_tts`, `normalize_extension`. All return `Outcome<T>`. |
| [`factory/`](./factory/) | Provider traits and implementations. `entry.rs` has `create_*_provider`, `default_*_provider`, `DEFAULT_STT_MODEL`, `DEFAULT_PIPER_VOICE`; `traits.rs` the traits; `stt_providers.rs` (`CloudSttProvider`, `ExternalSttProvider`); `tts_providers.rs` (`CloudTtsProvider`, `PiperTtsProvider`, `ExternalTtsProvider`); `helpers.rs` (`split_slug_model`, `effective_*_provider`, slug lookup in `config.voice_providers`). |
| [`schemas/`](./schemas/) | Controller schemas and handlers. `registry.rs` lists the methods and chains in the [`live`](./live) controllers; `params.rs` and `helpers.rs` parse input; `handlers.rs` with `handlers/transcribe_tts.rs` and `handlers/provider_server.rs` hold the `handle_voice_*` and `handle_overlay_stt_notify` functions. |
| [`cloud_transcribe.rs`](./cloud_transcribe.rs) | Auth adapter for hosted STT: resolves the backend credential and calls `tinyinference_voice::cloud` against `/openai/v1/audio/transcriptions`. |
| [`local_speech.rs`](./local_speech.rs) | Config adapter for Piper: resolves the binary and voice through the local runtime and calls `tinyinference-voice`. |
| [`postprocess.rs`](./postprocess.rs) | Config adapter for LLM transcript cleanup (`cleanup_transcription`). |
| [`streaming.rs`](./streaming.rs) | `/ws/dictation` handler (`handle_dictation_ws`). Compiled only with both `voice` and `http-server`. |
| [`reply_speech.rs`](./reply_speech.rs) | Agent reply synthesis through the backend speech endpoint (`synthesize_reply`, `ReplySpeechOptions`). |
| [`server.rs`](./server.rs), [`server/`](./server/) | The `VoiceServer` dictation runtime: hotkey loop, recording lifecycle, gates, delivery, global singleton. [README](server/README.md). |
| [`always_on.rs`](./always_on.rs), [`always_on/`](./always_on/) | VAD-driven continuous listening, screen-lock pause, wake word and intent routing. [README](always_on/README.md). |
| [`audio_capture.rs`](./audio_capture.rs) | Microphone recording to 16 kHz mono WAV (`start_recording`, `RecordingHandle`, `RecordingResult`, `list_input_devices`) with peak RMS and the host microphone-permission policy. The `cpal` stream is `tinyvoice::capture`. |
| [`dictation_listener.rs`](./dictation_listener.rs) | The two broadcast channels and host event-forwarding lifecycle over TinyVoice's native listener. |
| [`bus.rs`](./bus.rs) | `publish_ptt_transcript_committed`. |
| `live/` | Live voice agents: `providers.rs`, `session.rs`, `ws.rs` (`http-server` only), `persist.rs`, `ops.rs` / `schemas.rs` (`voice.live_*`), `types.rs`, `error.rs`, and `prompt.md` (voice guidance for the model). |
| [`realtime.rs`](./realtime.rs) | `mint_voice_agent_signed_url`: a short-lived signed WebSocket URL from the backend's `/voice-agent/get-signed-url`. The provider API key never leaves the server. |
| [`realtime_harness.rs`](./realtime_harness.rs), [`realtime_harness/`](./realtime_harness/) | `voice:harness` turn handler: `prompt.rs` (prompt extraction), `agent.rs` (per-turn orchestrator), `chat_delivery.rs` (socket emits, deferred chat delivery), `turn_handler.rs`. |
| [`cli.rs`](./cli.rs) | `openhuman voice` / `openhuman dictate`: a blocking standalone dictation server. Domain-owned because it never returns and does not fit the controller registry. |
| [`compile_status.rs`](./compile_status.rs) | `VOICE_COMPILED_IN`, compiled in both feature states. |
| [`stub.rs`](./stub.rs) | The disabled-voice facade, compiled only when `voice` is off. |

## Key types and entry points

- `SttProvider` / `TtsProvider` ([`factory/traits.rs`](./factory/traits.rs)) are the traits every
  backend implements. `SttResult` carries `text` and the `provider` that
  produced it.
- `create_stt_provider` / `create_tts_provider` ([`factory/entry.rs`](./factory/entry.rs)) map a
  provider string to an implementation. Add a new engine as a branch here plus a
  sibling module; the doc comment in `entry.rs` describes how Kokoro would be
  added.
- `effective_stt_provider` / `effective_tts_provider` ([`factory/helpers.rs`](./factory/helpers.rs))
  decide which provider string applies for a given config.
- `voice_transcribe`, `voice_transcribe_bytes`, `voice_tts`, `voice_status`
  (`ops.rs`) are the business operations behind the core RPCs.
- `VoiceServer` ([`server/runtime.rs`](./server/runtime.rs)) with `global_server`, `try_global_server`,
  `start_if_enabled` and `run_standalone` ([`server/singleton.rs`](./server/singleton.rs)).
- `always_on::start_if_enabled` / `always_on::stop` start and stop continuous
  listening; the config controller calls `start_if_enabled` again after
  voice-server settings are saved.
- `synthesize_reply` ([`reply_speech.rs`](./reply_speech.rs)) and `transcribe_cloud`
  ([`cloud_transcribe.rs`](./cloud_transcribe.rs)) are the direct hosted-backend entry points.
- `handle_live_voice_ws` ([`live/ws.rs`](./live/ws.rs)) and `handle_dictation_ws`
  (`streaming.rs`) are the WebSocket handlers the RPC crate mounts.
- `handle_voice_harness_turn` (`realtime_harness/`) runs one realtime harness
  turn.

## RPC / CLI surface

All methods are in the `voice` namespace and registered through
`all_voice_registered_controllers`, which also chains in the `live` controllers.

| Method | Purpose |
| --- | --- |
| `voice.status` | Availability without running anything. STT is available if the effective provider constructs and (for non-hosted slugs) has a credential; TTS if the Piper binary and voice model resolve. |
| `voice.transcribe` | Transcribe a file path, with optional LLM cleanup. |
| `voice.transcribe_bytes` | Transcribe raw bytes (via a temp file), with hallucination filter and cleanup. |
| `voice.tts` | Synthesize speech to a file with Piper. |
| `voice.reply_synthesize` | Synthesize an agent reply through the effective TTS provider; returns base64 audio and visemes. |
| `voice.cloud_transcribe` | Transcribe base64 audio through the hosted STT proxy (back-compat path). |
| `voice.stt_dispatch` | Factory-dispatched STT (`cloud` or `<slug>:<model>`); returns `{ text, provider }`. |
| `voice.tts_dispatch` | Factory-dispatched TTS (`cloud`, `piper`, `<slug>:<voice>`); returns a reply-speech result. |
| `voice.set_providers` | Persist the STT/TTS provider and model or voice into `config.local_ai.*`. |
| `voice.update_provider_settings` | Persist the voice provider registry and routing strings (the voice twin of the inference model settings). |
| `voice.list_models` | Models or voices for a provider (static presets for built-in slugs). |
| `voice.test_provider` | Test a provider with a silent WAV (STT), "Hello" (TTS), or a key-only validation. `validate_only` is a dry run for both workloads and accepts an `api_key` to check a candidate credential without storing it. |
| `voice.server_start`, `voice.server_stop`, `voice.server_status` | Control the global dictation server. |
| `voice.overlay_stt_notify` | Bridge chat-button STT state transitions onto the dictation and transcription channels. |
| `voice.agent_signed_url` | Mint a signed URL for the hosted ElevenLabs agent. The live `elevenlabs-hosted` provider now mints it in-core, so the frontend does not call this. |
| `voice.live_providers` | Live providers with readiness, kind (hosted or BYOK), key slug, voices, languages and the default. |
| `voice.live_settings_get`, `voice.live_settings_set` | Read or patch `[voice_live]`: default provider and per-provider model, voice and language. |
| `voice.live_test_provider` | Open a live session, wait for `Ready`, close; returns `{ ok, latency_ms, error }`. |

The CLI adapter in [`cli.rs`](./cli.rs) is registered in `core/all.rs` without a feature
gate, so a voice-less build answers `openhuman voice` with a "voice disabled"
error from the stub.

## Persistence

There is no `store.rs`. Settings live in the shared TOML `Config` and are
written through the config domain: `config.local_ai.{stt,tts}_provider`,
`config.local_ai.{stt_model_id,tts_voice_id}`, top-level
`config.{stt,tts}_provider`, `config.voice_server.*` (`stt_engine`,
`always_on_enabled`, `wake_word`, hotkey, gates), `config.voice_providers` (the
registry, typed in `config/schema/voice_providers.rs`) and `[voice_live]`. The
dictation server keeps only in-memory state (state machine, transcription
count, rolling recent-transcript buffer) behind a `OnceCell` singleton.

## Boundaries

- `tinyinference-voice` (in `vendor/tinyagents/vendor/tinyinference`) owns hosted
  STT transport, Piper execution, cleanup and streaming PCM mechanics, and the
  reply-speech response types. `crate::inference` supplies the local runtime
  config and provider policy.
- The `tinyvoice` native module (`vendor/tinyvoice`, reached through
  `crate::modules::voice` with its contract in `tinyvoice-bus`) owns frame
  preparation, resampling, energies, WAV encoding, the VAD session, wake-word
  detection, command/intent routing and `is_hallucinated`. The `tinyvoice`
  module owns native hotkey parsing, event capture and listener cleanup. The
  library capture feature still owns the `cpal` stream until the module capture
  lifecycle cut lands; this folder owns microphone permission policy.
- The compiled TinyComputer module owns text insertion, focused-text inspection,
  permission decisions and the macOS Swift Globe listener. OpenHuman uses
  `tinycomputer-bus` vocabulary and confidential focus/paste calls. Globe events
  arrive in bounded replayable batches; the host acknowledges delivered batches
  and closes the lease on cancellation or server stop. Overflow resets activation
  and requires a physical release before the next press can rearm it.
- `tinyliveagents` owns live provider wire protocols and `tinyagents-live` owns
  live tool execution. The tool middleware is `agent::tinyagents::live_harness`.
- `crates/openhuman-rpc` mounts and authenticates `/ws/dictation` and
  `/ws/live-voice` (bearer header or `?token=`, plus the origin allowlist) and
  forwards the broadcast channels over Socket.IO. Do not add auth checks inside
  the handlers here.
- Backend credentials come from
  `security::credentials::session_support::resolve_backend_credential` (session
  JWT or TinyHumans API key) and backend URLs from `Config.resolved_endpoints`. The core
  does not hold hosted URLs itself.
- Config shape lives under `config/schema/` (`voice_server.rs`,
  `voice_providers.rs`); migrations live in `config/migrations/`.

Callers outside this folder include `core/all.rs` (controller and CLI
registration), `web_chat/run_task.rs` (reply speech, PTT events),
`channels/host/adapters.rs` (channel STT and reply synthesis),
`security/credentials/ops/gated_services.rs` (starts and stops the dictation
server, listener and always-on mode when gating credentials change),
`config/schemas/controllers/voice.rs`, `desktop/overlay/`,
`inference/host_runtime/service/speech.rs` and `crates/openhuman-app/src/lib.rs` (asserts
`VOICE_COMPILED_IN`).

## Gotchas

- The desktop product enables `voice` explicitly; contributor defaults leave
  it off. `pub mod voice` always compiles as a facade. With the feature off, [`stub.rs`](./stub.rs) replaces the real modules and mirrors
  the surface other code depends on (`server`, `dictation_listener`,
  `always_on`, `streaming`, `live::ws`, `reply_speech`, `cloud_transcribe`,
  `cli`, `create_stt_provider`, `effective_stt_provider`, `SttProvider`,
  `publish_ptt_transcript_committed`) with no-op or disabled-error bodies.
  Signature drift is caught by the disabled build
  (`cargo check --no-default-features --features "<all-but-voice>"`), so change
  both sides together. `VOICE_COMPILED_IN` is ungated so the desktop shell can
  assert at compile time that it did not get the stub (#4901).
- `voice` pulls in `inference`, `modules` and the `tinyvoice-bus` contract. `streaming` additionally needs `http-server`.
- macOS hotkeys (#2677): rdev's former CGEventTap callback called
  `TSMGetInputSourceProperty` off the main thread, which crashes with
  `EXC_BREAKPOINT` on macOS 26. On macOS `dictation_listener::start_if_enabled`
  is a no-op and the dictation server accepts only the `fn` (Globe) key through
  the Swift listener; any other key returns an error. Other platforms use the
  TinyVoice module's native listener.
- Approval classification differs by path. Reply speech is internal: if it is
  ever wrapped in a `Tool`, `external_effect()` must stay `false` so TTS never
  prompts (#1339, #1206). Realtime harness and live turns are external-channel
  origins because they start as user speech.
- Dispatch handlers default to `DEFAULT_PIPER_VOICE` only when the active TTS
  provider is `piper`. Sending a Piper voice id to a cloud or external endpoint
  is invalid.
- `reply_speech` has an env-gated test seam: when its env var is set to `1` or
  `true`, `synthesize_reply` records the text and returns a stub without
  calling the backend. It is env-gated rather than `cfg(test)` so integration
  tests in `tests/` can use it.
- Live sessions save spoken turns to the thread for the user but do not yet
  write them to the agent's session transcript, so a later typed turn's model
  does not see them. The live model does see recent typed messages through its
  prompt.

## Tests

Tests sit beside their modules as `<module>_tests.rs` (for example
[`ops_tests.rs`](./ops_tests.rs), [`factory/factory_tests.rs`](./factory/factory_tests.rs), [`live/session_tests.rs`](./live/session_tests.rs)), wired with
`#[path = ...]`. Run them with `cargo test -p openhuman voice::` or
`pnpm debug rust voice`. Check the disabled build with
`cargo check --no-default-features` plus every product feature except `voice`.

## Further reading

- [Voice tools](../../../../docs/gitbooks/en/features/native-tools/voice.md)
- [tinyvoice submodule](../../../../vendor/tinyvoice/README.md)
- [Chat](../../../../docs/gitbooks/en/features/chat.md)
