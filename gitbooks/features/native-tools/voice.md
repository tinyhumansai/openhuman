---
description: >-
  Native voice - speech-to-text in, text-to-speech out, mascot lip-sync,
  and a live Google Meet agent that listens and speaks.
icon: microphone
---

# Voice

OpenHuman is voice-first when you want it to be. STT, TTS, and the live Google Meet agent are part of the core, not a third-party plugin.

## Speech-to-text

- **Hotkey** - push-to-talk and toggle modes.
- **Audio capture** - cross-platform mic capture with voice-activity detection.
- **Streaming transcription** - words appear as you speak.
- **Hallucination filter** - strips well-known artefacts ("Thanks for watching", silence-induced phrases).
- **Postprocess** - punctuation, capitalisation, dictation cleanup.

Dictation can replace the active text input on your desktop, or be sent straight into a chat with the agent.

## Dictate into a chat draft

On Chat, select **Dictate** to record into the text composer. You can keep editing while recording. Select **Finish dictation** to transcribe the clip through your configured speech provider and append one final transcript to the draft. Review or edit it, then select **Send**.

Select Finish within one minute. At that limit, capture stops and the clip is discarded with a timeout message; it is not uploaded or transcribed.

**Discard dictation** or Escape discards the recording or pending transcript. Switching threads, switching to Voice mode, or leaving the composer cancels dictation. The microphone stops when recording ends. Dictation appears when microphone capture and the core's speech provider are available.

**Voice mode**, shown with a waveform icon, opens the microphone composer and sends its transcript as a conversation message. **Dictate**, shown with a microphone icon, adds text to your editable draft.

## Text-to-speech

Reply speech routes through a hosted TTS model. The agent's responses can be spoken back in a voice you pick, with natural timing and prosody. Voice selection is configurable per user, and the mascot avatar lip-syncs to the audio stream via a viseme map.

## Live Google Meet agent

OpenHuman's flagship voice integration:

- Joins a Google Meet via the embedded webview.
- Streams audio out to STT in real time, transcribes everyone in the call, and writes structured notes into the [Memory](../memory.md) as the meeting progresses.
- When you ask it to speak (or it decides it has something useful to add), it generates audio through the TTS model and **plays it back into the meeting as an outbound camera/mic stream**, so other participants actually hear it.

## Privacy

- Audio capture is local. Streaming STT goes through the OpenHuman backend; no recording is retained beyond the live transcript.
- TTS audio is streamed and discarded - nothing stored.
- Meeting transcripts are stored as conversations in your memory engine, when memory is on.

## See also

- [Memory](../memory.md) - where Meet transcripts and notes live.
- [Automatic Model Routing](../model-routing/) - Meet's brain uses `hint:fast` for low-latency conversational turns.
