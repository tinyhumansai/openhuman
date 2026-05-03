import { useEffect, useRef, useState } from 'react';

import { subscribeChatEvents } from '../../services/chatService';
import type { MascotFace } from './Mascot';
import { lerpViseme, VISEMES, type VisemeShape } from './Mascot/visemes';
import { type PlaybackHandle, playBase64Audio } from './voice/audioPlayer';
import { synthesizeSpeech } from './voice/ttsClient';
import { findActiveFrame, oculusVisemeToShape } from './voice/visemeMap';

/** ms the mouth holds the target viseme before decaying back to rest. */
const VISEME_DECAY_MS = 180;

/**
 * Pick a viseme from the trailing letter of a text delta. Heuristic — we
 * have no phoneme data — but it gives the mouth varied motion that tracks
 * the streaming text instead of just opening and closing the same way.
 */
export function pickViseme(delta: string): VisemeShape {
  const ch = delta
    .replace(/[^a-zA-Z]/g, '')
    .slice(-1)
    .toLowerCase();
  switch (ch) {
    case 'a':
      return VISEMES.A;
    case 'e':
      return VISEMES.E;
    case 'i':
    case 'y':
      return VISEMES.I;
    case 'o':
      return VISEMES.O;
    case 'u':
    case 'w':
      return VISEMES.U;
    case 'm':
    case 'b':
    case 'p':
      return VISEMES.M;
    case 'f':
    case 'v':
      return VISEMES.F;
    default:
      return VISEMES.E;
  }
}

export interface UseHumanMascotOptions {
  /** When true, post-stream replies are sent to ElevenLabs and the mouth
   *  follows the returned viseme timeline while the audio plays. */
  speakReplies?: boolean;
}

/**
 * Drives the mascot's face/mouth from chat events, with three phases:
 * - inference_start → thinking
 * - text_delta → speaking, pseudo-lipsync from the trailing letter of each delta
 * - chat_done (with `speakReplies`) → speaking, real visemes from TTS audio
 *   for the full response; falls back to neutral when audio ends or fails
 */
export function useHumanMascot(options: UseHumanMascotOptions = {}): {
  face: MascotFace;
  viseme: VisemeShape;
} {
  const { speakReplies = false } = options;
  const speakRef = useRef(speakReplies);
  speakRef.current = speakReplies;

  const [face, setFace] = useState<MascotFace>('normal');
  const targetRef = useRef<VisemeShape>(VISEMES.REST);
  const lastDeltaAtRef = useRef(0);

  // TTS playback state — non-null while audio is mid-flight.
  const playbackRef = useRef<PlaybackHandle | null>(null);
  const visemeFramesRef = useRef<{ viseme: string; start_ms: number; end_ms: number }[]>([]);
  const visemeCursorRef = useRef(0);
  // Monotonic counter — only the latest startTtsPlayback's callbacks may
  // mutate idle state; older invocations bail out.
  const playbackSeqRef = useRef(0);

  const [, force] = useState(0);

  useEffect(() => {
    const unsub = subscribeChatEvents({
      onInferenceStart: () => setFace('thinking'),
      onTextDelta: e => {
        // Pseudo-lipsync only kicks in if no real audio is playing.
        if (playbackRef.current) return;
        setFace('speaking');
        targetRef.current = pickViseme(e.delta);
        lastDeltaAtRef.current = window.performance.now();
      },
      onDone: e => {
        if (!speakRef.current || !e.full_response?.trim()) {
          setFace('normal');
          return;
        }
        // Fire-and-forget — startTtsPlayback owns its cleanup via finally.
        void startTtsPlayback(e.full_response).catch(() => {});
      },
      onError: () => {
        // Bump seq to invalidate any in-flight startTtsPlayback awaiters.
        playbackSeqRef.current++;
        playbackRef.current?.stop();
        playbackRef.current = null;
        visemeFramesRef.current = [];
        setFace('normal');
      },
    });
    return () => {
      unsub();
      // Same — invalidate in-flight callbacks before tearing down.
      playbackSeqRef.current++;
      playbackRef.current?.stop();
      playbackRef.current = null;
    };
  }, []);

  async function startTtsPlayback(text: string): Promise<void> {
    // Cancel any in-flight playback so its handle.ended callback can't reset
    // state belonging to the new run.
    playbackRef.current?.stop();
    playbackRef.current = null;
    visemeFramesRef.current = [];
    visemeCursorRef.current = 0;
    const seq = ++playbackSeqRef.current;
    const isStillCurrent = () => playbackSeqRef.current === seq;

    try {
      setFace('thinking');
      const tts = await synthesizeSpeech(text);
      if (!isStillCurrent()) return;
      visemeFramesRef.current = tts.visemes ?? [];
      visemeCursorRef.current = 0;
      const handle = await playBase64Audio(tts.audio_base64, tts.audio_mime ?? 'audio/mpeg');
      if (!isStillCurrent()) {
        handle.stop();
        return;
      }
      playbackRef.current = handle;
      setFace('speaking');
      try {
        await handle.ended;
      } catch {
        // Promise rejects when stop() is called — fall through to cleanup.
      }
    } finally {
      if (isStillCurrent()) {
        playbackRef.current = null;
        visemeFramesRef.current = [];
        setFace('normal');
      }
    }
  }

  // RAF loop while we're speaking (either pseudo-lipsync decay or audio-driven).
  useEffect(() => {
    if (face !== 'speaking') return;
    let raf = 0;
    const loop = () => {
      force(t => t + 1);
      raf = window.requestAnimationFrame(loop);
    };
    raf = window.requestAnimationFrame(loop);
    return () => window.cancelAnimationFrame(raf);
  }, [face]);

  let viseme: VisemeShape = VISEMES.REST;
  const playback = playbackRef.current;
  if (playback) {
    const ms = playback.currentMs();
    if (ms >= 0) {
      const { frame, cursor } = findActiveFrame(
        visemeFramesRef.current,
        ms,
        visemeCursorRef.current
      );
      visemeCursorRef.current = cursor;
      viseme = frame ? oculusVisemeToShape(frame.viseme) : VISEMES.REST;
    }
  } else if (face === 'speaking') {
    const since = window.performance.now() - lastDeltaAtRef.current;
    const decay = Math.max(0, Math.min(1, since / VISEME_DECAY_MS));
    viseme = lerpViseme(targetRef.current, VISEMES.REST, decay);
  }

  return { face, viseme };
}
