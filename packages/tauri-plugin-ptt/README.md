# tauri-plugin-ptt

Push-to-talk + TTS plugin for Tauri v2, targeting iOS.

Wraps `AVAudioEngine` + `Speech.framework` (STT) + `AVSpeechSynthesizer` (TTS).
On non-iOS targets all commands return a `NotSupported` error so the
desktop build is not affected.

## Commands

| Command | Description |
|---|---|
| `start_listening` | Activate `AVAudioEngine` + `SFSpeechRecognizer`. Partial transcripts arrive as events. |
| `stop_listening` | Deactivate and return final transcript text. |
| `speak` | Enqueue an `AVSpeechSynthesizer` utterance. |
| `cancel_speech` | Stop current utterance immediately. |
| `list_voices` | List all `AVSpeechSynthesisVoice.speechVoices()`. |

## Events

| Event | Payload | Description |
|---|---|---|
| `ptt://transcript-partial` | `{ text: string }` | Live partial result while recording. |
| `ptt://transcript-final` | `{ text: string }` | Final result after `stop_listening`. |
| `ptt://tts-started` | `{ utteranceId: string }` | TTS synthesis began. |
| `ptt://tts-ended` | `{ utteranceId: string; finished: boolean }` | TTS ended (false=cancelled). |
| `ptt://error` | `{ code: string; message: string }` | Async error (permission, interruption, etc.). |

### Error codes

| Code | Trigger |
|---|---|
| `permission_denied` | Microphone or speech recognition access denied. |
| `interrupted` | Phone call or system audio interrupted the session. |
| `route_changed` | BT headset disconnected mid-recording. |
| `audio_error` | AVAudioEngine failure. |
| `recognition_error` | SFSpeechRecognizer transcription failure. |

## Required iOS permissions (Info.plist)

```xml
<key>NSMicrophoneUsageDescription</key>
<string>Used for push-to-talk voice messages.</string>
<key>NSSpeechRecognitionUsageDescription</key>
<string>Used to transcribe your voice to text.</string>
```

## Manual testing checklist

The Swift layer cannot be unit-tested in CI (requires iOS toolchain + simulator).
Test on a physical device or simulator against the following:

- [ ] Permissions dialog appears on first `startListening` call.
- [ ] Partial transcripts update while speaking; final transcript matches.
- [ ] Hold button to record, release to stop, chat message is sent with transcript.
- [ ] TTS plays through speaker by default when iPhone is held away from ear.
- [ ] BT headset routes audio correctly; disconnecting mid-recording stops gracefully.
- [ ] App backgrounded mid-record produces a final transcript and stops cleanly.
- [ ] Phone call interruption emits `ptt://error` with `code: interrupted`.
- [ ] `cancelSpeech` during TTS emits `tts-ended` with `finished: false`.
- [ ] `listVoices` returns non-empty list of `AVSpeechSynthesisVoice` entries.

## Architecture

```
JS (MascotScreen)
  ↓ invoke / listen
Rust (commands.rs)
  ↓ PluginHandle::run_mobile_plugin
Swift (PTTPlugin.swift)
  ↓
  PTTRecorder — AVAudioEngine + SFSpeechRecognizer
  PTTSpeaker  — AVSpeechSynthesizer
  AudioSessionManager — AVAudioSession lifecycle + notifications
```
