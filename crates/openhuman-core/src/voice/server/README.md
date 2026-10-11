# voice/server

The `VoiceServer` dictation runtime: hotkey press, record, transcribe, insert
text. It is the implementation behind `voice::server` (see the parent
[README](../README.md)), and can run embedded in the core process or
standalone through the `openhuman voice`/`openhuman dictate` CLI subcommand.

## Responsibilities

- Listen for a configurable hotkey and turn press/release events into audio recordings.
- Hand each finished recording to a background pipeline that gates on minimum duration, silence, and hallucinated output, then transcribes and delivers the result.
- Track running state (`Stopped`/`Idle`/`Recording`/`Transcribing`), a transcription counter, and the last error, exposed through `VoiceServerStatus`.
- Keep a rolling buffer of recent transcripts to bias the STT engine's `initial_prompt` for continuity across consecutive recordings.
- Provide a process-global singleton so RPC status/stop calls and the embedded auto-start path always act on the same running instance.

## Key files

| File | Role |
| --- | --- |
| [`types.rs`](./types.rs) | `ServerState`, `VoiceServerStatus`, `VoiceServerConfig` (hotkey, activation mode, skip_cleanup, context, min duration, silence threshold, custom dictionary), plus the `DEFAULT_SILENCE_THRESHOLD` (0.002 RMS, matching OpenWhispr's default), `MAX_RECENT_TRANSCRIPTS` (5), and `MAX_INITIAL_PROMPT_CHARS` (500) tunables. |
| [`runtime.rs`](./runtime.rs) | `VoiceServer`: the hotkey event loop. Starts the platform hotkey listener, tracks in-progress and pending recordings, handles the race between a buffered release event and recording setup still in flight, and spawns `process_recording_bg` off the event loop so rapid consecutive presses are never missed. Owns `run`/`stop`/`status`. |
| [`hotkey_listener.rs`](./hotkey_listener.rs) | Picks the platform-appropriate listener. Uses `rdev` everywhere except the macOS `Fn`/Globe key, which goes through a Swift-based globe listener inside the compiled TinyComputer module, with acknowledged batches and explicit lease cleanup instead, because `rdev`'s `CGEventTap` callback calls `TSMGetInputSourceProperty` off the main thread and macOS 26 kills the process for that (#2677). Any other hotkey is rejected on macOS with a message pointing at `hotkey = "fn"`. |
| [`pipeline.rs`](./pipeline.rs) | `process_recording_bg`: stops the recording, applies the duration/silence/hallucination gates, builds the `initial_prompt` from the custom dictionary and recent transcripts, transcribes via `crate::voice::voice_transcribe_bytes`, and delivers the text either over Socket.IO (when the focused app is OpenHuman itself) or by pasting into the external app via `text_input::insert_text`. |
| [`singleton.rs`](./singleton.rs) | `global_server`/`try_global_server` (the process-global `OnceCell<Arc<VoiceServer>>`), `start_if_enabled` (embedded auto-start, gated on `config.voice_server.auto_start`), and `run_standalone` (the blocking CLI entry point, which deliberately does not register in the global singleton so CLI-started instances stay isolated from the core RPC lifecycle). |

## How it fits

`server.rs` in the parent `voice` module re-exports this submodule's public
surface (`VoiceServer`, `global_server`, `try_global_server`,
`start_if_enabled`, `run_standalone`, `ServerState`, `VoiceServerConfig`,
`VoiceServerStatus`) and is itself gated behind the `voice` Cargo feature (see
the compile-time gate section in [../README.md](../README.md)). The RPC and
agent-tool surface for voice generally lives in `voice/schemas/` and
`voice/ops.rs`, not here; this module only owns the dictation runtime.

## Notes / gotchas

- Focus capture and microphone permission checks run through async TinyComputer calls; device setup then runs on a blocking thread (`audio_capture::start_recording`) so the event loop stays responsive to a `Released` event that some keys, including `Fn`, fire almost immediately. A release or a second press that arrives during that setup is buffered as a stop intent and applied once the recording handle is ready, with a minimum post-setup recording window (1500ms) so very quick releases still capture real speech.
- `stop()` cancels the run-loop's `CancellationToken` and polls for up to 5 seconds for the state to reach `Stopped`, since a fast logout/login cycle should not see a stale `Idle`/`Recording` state and skip a restart.
- State updates carry a `generation` counter so a stale background pipeline task from a superseded recording cannot overwrite the state of a newer one.

## Further reading

- [Parent module (`voice`)](../README.md)
- [Voice tools](../../../../../gitbooks/features/native-tools/voice.md)
- [tinyvoice submodule](../../../../../vendor/tinyvoice/README.md)
- [Chat](../../../../../gitbooks/features/chat.md)
