---
description: >-
  Speech-to-text in, text-to-speech out, mascot lip-sync, and a live voice
  agent you can interrupt mid-sentence.
icon: microphone
---

# Voice

Voice is an optional core capability. The Cargo `voice` feature is off in contributor builds and on in the desktop product; the runtime component is enabled by default. Dictation, spoken replies and the live voice agent use the same core voice domain.

## Speech-to-text

- **Recording.** Start a microphone recording from the composer and stop it to transcribe.
- **Hallucination filter.** Strips known artifacts such as "Thanks for watching" and phrases invented from silence.
- **Postprocessing.** Punctuation, capitalization and dictation cleanup.

To send a voice message when recording ends, use **Voice mode**. Desktop global voice shortcuts are no longer part of the product.

## Dictate into a chat draft

On Chat, select **Dictate** to record into the text composer. You can keep editing while recording. Select **Finish dictation** to transcribe the clip through your configured speech provider and append one final transcript to the draft. Review or edit it, then select **Send**.

Select Finish within one minute. At that limit, capture stops and the clip is discarded with a timeout message; it is not uploaded or transcribed.

**Discard dictation** or Escape discards the recording or pending transcript. Switching threads, switching to Voice mode, or leaving the composer cancels dictation. The microphone stops when recording ends. Dictation appears when microphone capture and the core's speech provider are available.

**Voice mode**, shown with a waveform icon, opens the microphone composer and sends a voice message with its transcript. **Dictate**, shown with a microphone icon, adds text to your editable draft.

## Text-to-speech

Replies are spoken through a hosted TTS model. The agent can speak back in a voice you pick, with natural timing. Voice selection is set per user, and the mascot lip-syncs to the audio through a viseme map.

## Talk to Tiny live

Click Tiny, the mascot in the chat composer, and talk. Tiny listens and answers out loud in real time, and you can cut in mid-sentence. It uses the same tools as typed chat under the same tool policy, and anything that needs your approval asks in the conversation. What you both say is saved to the open conversation, and Tiny knows what you were just typing about.

Choose how Tiny's voice runs under **Connections → Voice agents**:

| Provider                      | Setup                     | Notes                                                                                                                                        |
| ----------------------------- | ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| Gemini Live (TinyHumans)      | None (default)            | One model that hears and speaks. Billed through your TinyHumans balance.                                                                     |
| ElevenLabs Agent (TinyHumans) | None                      | Hosted ElevenLabs voice with OpenHuman as its brain.                                                                                         |
| Gemini Live (Google API key)  | Your Google AI Studio key | Talks to Google directly.                                                                                                                    |
| Sarvam AI                     | Your Sarvam key           | Indian languages (Hindi, Tamil, Bengali and more, or automatic detection). Sarvam's speech recognition, chat model and voices work together. |

Each card has a **Test** button that opens a short session to check the provider. You can also pick the voice, and the language where the provider supports it.

## Privacy

- Audio capture is local. Where the audio goes next depends on the provider you picked. Managed routes send it to the OpenHuman backend. A bring-your-own-key provider such as Gemini Live on a Google API key talks to that vendor directly. Either way, no recording is kept beyond the live transcript.
- TTS audio is streamed and discarded. Nothing is stored.
- What you and the agent say in a live session is saved to the open conversation. When memory is on, it reaches your memory engine on the same terms as typed chat.

There is no local speech-to-text engine. Speech-to-text is either the hosted route or a third-party API you bring a key for. Text-to-speech still has a local option (Piper).

## Google Meet

The live Google Meet agent has been removed. Nothing in the shipped build joins a meeting. The second mascot and its per-mascot voice remain as [mascot](../mascot/README.md) settings.

## See also

- [The mascot](../mascot/README.md): the face that lip-syncs to this audio.
- [Memory](../memory.md): where a spoken conversation ends up.
- [Automatic model routing](../model-routing/README.md): live turns want `hint:fast`.
