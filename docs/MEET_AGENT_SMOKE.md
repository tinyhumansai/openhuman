# Meet-agent live loop — smoke test runbook

End-to-end validation that the agent hears, thinks, and speaks on a
real Google Meet call. Two laptops are easiest (Laptop A runs OpenHuman
+ joins the Meet as the agent; Laptop B is the human host who creates
the call and listens to the agent's voice).

## Pre-flight

1. Sign in to OpenHuman so a backend session token exists. Without
   it, all three brain stages (STT/LLM/TTS) silently fall back to
   stubs and you'll only hear a 200 ms tone — useful for plumbing
   smoke but not the real loop.
2. Ensure the vendored `tauri-cef` submodule is on
   `feat/openhuman-audio-handler` (or whatever branch carries the
   `audio` module — see `app/src-tauri/vendor/tauri-cef`).
3. `pnpm tauri dev` in the repo root.

## Steps

1. **Laptop B**: create a Meet call at <https://meet.google.com/new>,
   stay in the lobby.
2. **Laptop A**:
   - Open OpenHuman → Intelligence → Calls.
   - Paste the Meet URL, set display name (e.g. "OpenHuman Agent").
   - Click *Join*.
   - A dedicated CEF window opens. The window title bar reads
     "Meet — OpenHuman Agent".
3. **Laptop B**: admit the agent from the lobby.
4. **Laptop B**: speak a question into your mic. Examples:
   - "Hey, can you confirm you can hear me?"
   - "What's the weather like in Paris today?"

## What to watch for

### Listen path (Meet → agent)

- Tail the file logs (`~/Library/Application Support/OpenHuman/logs/`):

  ```text
  [meet-audio] cef stream start request_id=… hz=48000 channels=2 …
  [meet-audio] forward channel push (…)
  [meet-agent-rpc] handle_push_listen_pcm turn_started=true (when you stop talking)
  [meet-agent] STT request_id=… text_chars=…
  [meet-agent] turn done request_id=… reply_chars=… synth_samples=…
  ```

- If `cef stream start` never logs, the per-browser CEF audio handler
  isn't installed. Check that `tauri_runtime_cef::audio::register_audio_handler`
  matched the meet URL prefix.

### Speak path (agent → Meet)

- Inspect the embedded Meet page's console (right-click → Inspect; or
  attach via the CDP port 19222 on Laptop A): you should see
  `[openhuman-audio-bridge] feed failed: …` only on errors.
- Run `window.__openhumanAudioBridgeInfo()` in the console:

  ```json
  { "installed": true, "sample_rate": 16000, "audio_context_state": "running",
    "next_start_time": 12.3, "destination_track_count": 1 }
  ```

- **Laptop B**: you should hear the agent's reply through Meet, with
  the agent's tile lighting up the "speaking" indicator.

### Mascot webcam

- Laptop B sees the OpenHuman mascot SVG in the agent's tile.
  Confirms `--use-file-for-fake-video-capture` is still active (the
  speak path doesn't break it).

## Things that should NOT happen

- macOS prompt for screen recording / microphone permission.
- macOS prompt for installing a system audio driver / kext.
- The OpenHuman main window's mic indicator turning on (we tap CEF's
  audio at the renderer level, not via the OS mic).

## Common failure modes

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| Heard event empty / "STT failure" | No backend session | Sign in |
| Spoke event present, no audio on Laptop B | Bridge install failed | Check `Page.reload` errored — devtools network |
| 1× turn fires, then nothing | VAD `in_utterance` flag stuck | Look for `EndOfUtterance` events; may need a longer hangover |
| Audio "robot voice" | Sample-rate mismatch — bridge says 16000 but TTS gave another rate | Confirm `output_format=pcm_16000` request was honored |
| `cef stream error` repeated | Renderer crashed | Check Chromium logs in the meet-call data dir |

## Cleanup

- Close the meet-call window. The window-destroyed handler tears down
  `meet_audio` (drops the audio handler registration → silences
  capture immediately, signals the speak pump → exits) and calls
  `openhuman.meet_agent_stop_session` which logs the listened/spoken
  totals.
- Per-call data dir is wiped automatically.
