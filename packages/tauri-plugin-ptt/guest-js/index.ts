/**
 * tauri-plugin-ptt — JS bindings for push-to-talk and TTS.
 *
 * Commands are routed via `plugin:ptt|<name>`.
 * Events arrive on the Tauri event bus from the Swift plugin:
 *   ptt://transcript-partial  { text: string }
 *   ptt://transcript-final    { text: string }
 *   ptt://tts-started         { utteranceId: string }
 *   ptt://tts-ended           { utteranceId: string; finished: boolean }
 *   ptt://error               { code: string; message: string }
 */
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

// ── Types ────────────────────────────────────────────────────────────────────

export interface TranscriptEvent {
  text: string;
  isFinal: boolean;
}

export interface VoiceInfo {
  id: string;
  name: string;
  lang: string;
}

export interface PttError {
  code: string;
  message: string;
}

export interface TtsEndedEvent {
  utteranceId: string;
  finished: boolean;
}

// ── Commands ─────────────────────────────────────────────────────────────────

/**
 * Begin a push-to-talk recording session.
 * Partial transcripts arrive as `ptt://transcript-partial` events.
 * Call `stopListening()` to end the session and get the final text.
 */
export async function startListening(): Promise<void> {
  await invoke('plugin:ptt|start_listening');
}

/**
 * Stop the active recording session.
 * Returns the final recognized text.
 * Also emits `ptt://transcript-final`.
 */
export async function stopListening(): Promise<TranscriptEvent> {
  return await invoke<TranscriptEvent>('plugin:ptt|stop_listening');
}

/**
 * Enqueue a TTS utterance via AVSpeechSynthesizer.
 * @param text - Text to speak.
 * @param opts.voiceId - Optional AVSpeechSynthesisVoice identifier.
 * @param opts.rate - Speed multiplier 0.5–2.0 (default 1.0).
 */
export async function speak(
  text: string,
  opts?: { voiceId?: string; rate?: number }
): Promise<void> {
  await invoke('plugin:ptt|speak', {
    text,
    voiceId: opts?.voiceId ?? null,
    rate: opts?.rate ?? null,
  });
}

/**
 * Immediately stop any in-progress TTS utterance.
 */
export async function cancelSpeech(): Promise<void> {
  await invoke('plugin:ptt|cancel_speech');
}

/**
 * List all on-device TTS voices from AVSpeechSynthesisVoice.speechVoices().
 */
export async function listVoices(): Promise<VoiceInfo[]> {
  return await invoke<VoiceInfo[]>('plugin:ptt|list_voices');
}

// ── Event subscriptions ──────────────────────────────────────────────────────

/**
 * Subscribe to live partial transcripts while the user speaks.
 */
export async function onTranscriptPartial(cb: (text: string) => void): Promise<UnlistenFn> {
  return listen<{ text: string }>('ptt://transcript-partial', e => cb(e.payload.text));
}

/**
 * Subscribe to the final transcript emitted after stopListening().
 */
export async function onTranscriptFinal(cb: (text: string) => void): Promise<UnlistenFn> {
  return listen<{ text: string }>('ptt://transcript-final', e => cb(e.payload.text));
}

/**
 * Subscribe to the TTS started event. Fires when synthesis begins for an utterance.
 */
export async function onTtsStarted(cb: (utteranceId: string) => void): Promise<UnlistenFn> {
  return listen<{ utteranceId: string }>('ptt://tts-started', e => cb(e.payload.utteranceId));
}

/**
 * Subscribe to the TTS ended event.
 * `finished` is false if the utterance was cancelled before completion.
 */
export async function onTtsEnded(
  cb: (utteranceId: string, finished: boolean) => void
): Promise<UnlistenFn> {
  return listen<TtsEndedEvent>('ptt://tts-ended', e =>
    cb(e.payload.utteranceId, e.payload.finished)
  );
}

/**
 * Subscribe to async PTT errors (permission denied, interruption, route change, etc.).
 */
export async function onError(cb: (err: PttError) => void): Promise<UnlistenFn> {
  return listen<PttError>('ptt://error', e => cb(e.payload));
}
