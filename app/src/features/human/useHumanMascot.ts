import debug from 'debug';
import { useEffect, useRef, useState } from 'react';
import { useSelector } from 'react-redux';

import { subscribeChatEvents } from '../../services/chatService';
import { selectEffectiveMascotVoiceId } from '../../store/mascotSlice';
import type { MascotFace } from './Mascot';
import { lerpViseme, VISEMES, type VisemeShape } from './Mascot/visemes';
import { type PlaybackHandle, playBase64Audio, swallowAudioStop } from './voice/audioPlayer';
import {
  proceduralVisemes,
  synthesizeSpeech,
  type VisemeFrame,
  visemesFromAlignment,
} from './voice/ttsClient';
import { findActiveFrame, oculusVisemeToShape } from './voice/visemeMap';

const mascotLog = debug('human:mascot');

/** ms the mouth holds the target viseme before decaying back to rest. */
const VISEME_DECAY_MS = 180;

/**
 * Heuristic — does this timeline contain at least one frame whose code maps
 * to a non-REST mouth shape? Used to detect the "backend shipped frames in
 * an unknown vocabulary" regression where the mouth visibly stops moving
 * because every viseme falls back to REST.
 */
function framesProduceMotion(frames: VisemeFrame[]): boolean {
  for (const f of frames) {
    const shape = oculusVisemeToShape(f.viseme);
    if (shape !== VISEMES.REST) return true;
  }
  return false;
}

/**
 * How long to hold a transient acknowledgement face (`happy`, `concerned`)
 * before decaying back to `idle`. Tuned to feel like a soft beat rather than
 * a snap. Exported for tests.
 */
export const ACK_FACE_HOLD_MS = 700;

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

type ConversationAckFace = Extract<MascotFace, 'happy' | 'confused' | 'concerned'>;
type ConversationAckEvent = { full_response?: string | null; reaction_emoji?: string | null };

const HAPPY_REACTION_EMOJIS = new Set(['✅', '🎉', '🙌', '😊', '😄', '👍', '💪']);
const CONFUSED_REACTION_EMOJIS = new Set(['🤔', '❓', '❔']);
const CONCERNED_REACTION_EMOJIS = new Set(['⚠️', '⚠', '🚨', '❌', '😕', '😟']);

const CONCERNED_TEXT_RE =
  /\b(sorry|apolog(?:y|ize|ise)|failed|failure|error|cannot|can't|unable|blocked|problem)\b/i;
const CONFUSED_TEXT_RE =
  /\b(not sure|unclear|ambiguous|clarify|which one|need more|can you confirm|maybe)\b/i;
const HAPPY_TEXT_RE = /\b(done|completed|fixed|success|successful|ready|all set|great|nice)\b/i;

/**
 * Map conversation-level meaning into the short acknowledgement face that
 * follows a completed turn. Runtime activity still owns thinking/speaking
 * states; this only decides the post-turn emotional beat.
 */
export function pickConversationAckFace(event: ConversationAckEvent): ConversationAckFace | null {
  const reaction = event.reaction_emoji?.trim();
  if (reaction) {
    if (HAPPY_REACTION_EMOJIS.has(reaction)) return 'happy';
    if (CONFUSED_REACTION_EMOJIS.has(reaction)) return 'confused';
    if (CONCERNED_REACTION_EMOJIS.has(reaction)) return 'concerned';
  }

  const text = event.full_response?.trim() ?? '';
  if (!text) return null;
  if (CONCERNED_TEXT_RE.test(text)) return 'concerned';
  if (CONFUSED_TEXT_RE.test(text)) return 'confused';
  if (HAPPY_TEXT_RE.test(text)) return 'happy';
  return null;
}

export interface UseHumanMascotOptions {
  /** When true, post-stream replies are sent to ElevenLabs and the mouth
   *  follows the returned viseme timeline while the audio plays. */
  speakReplies?: boolean;
  /** When true, force the mascot into a `listening` pose. Caller is responsible
   *  for setting this while the mic is hot (e.g. from voice dictation state). */
  listening?: boolean;
}

export interface UseHumanMascotResult {
  face: MascotFace;
  viseme: VisemeShape;
}

/**
 * Drives the mascot's face/mouth from agent + voice lifecycle events.
 *
 * Mapping (kept in one place so the visual model stays coherent):
 *
 * - `inference_start` → `thinking`
 * - `iteration_start` round > 1 or `tool_call` → `confused` (heavy reasoning)
 * - `tool_result success=false` → `concerned` (held briefly)
 * - `text_delta` → `speaking`, pseudo-lipsync from the trailing letter
 * - `chat_done` (no TTS) → message-aware ack face (held briefly), then `idle`
 * - `chat_done` (TTS enabled) → `thinking` while synthesizing → `speaking`
 *   with real visemes → message-aware ack face when the audio ends
 * - `chat_error`, TTS failure → `concerned` (held briefly), then `idle`
 * - `listening` option override → `listening` (highest priority)
 *
 * Errors and unavailable voice degrade cleanly: speech failures fall through
 * to text-only behavior and surface as a brief `concerned` beat.
 */
export function useHumanMascot(options: UseHumanMascotOptions = {}): UseHumanMascotResult {
  const { speakReplies = false, listening = false } = options;
  const speakRef = useRef(speakReplies);
  speakRef.current = speakReplies;
  const listeningRef = useRef(listening);
  listeningRef.current = listening;

  // Effective mascot voice id: resolves the manual override, the
  // locale-default toggle, and the build-time fallback into a single
  // string (see `selectEffectiveMascotVoiceId`). Mirrored into a ref so
  // the inner `startTtsPlayback` closure always reads the latest value
  // without having to re-create the callback on every re-render.
  const effectiveMascotVoiceId = useSelector(selectEffectiveMascotVoiceId);
  const mascotVoiceIdRef = useRef<string>(effectiveMascotVoiceId);
  mascotVoiceIdRef.current = effectiveMascotVoiceId;

  const [face, setFace] = useState<MascotFace>('idle');
  const targetRef = useRef<VisemeShape>(VISEMES.REST);
  const lastDeltaAtRef = useRef(0);
  const ackTimerRef = useRef<number | null>(null);

  // TTS playback state — non-null while audio is mid-flight.
  const playbackRef = useRef<PlaybackHandle | null>(null);
  const visemeFramesRef = useRef<{ viseme: string; start_ms: number; end_ms: number }[]>([]);
  const visemeCursorRef = useRef(0);
  // Monotonic counter — only the latest startTtsPlayback's callbacks may
  // mutate idle state; older invocations bail out.
  const playbackSeqRef = useRef(0);

  const [, force] = useState(0);

  function clearAckTimer() {
    if (ackTimerRef.current != null) {
      window.clearTimeout(ackTimerRef.current);
      ackTimerRef.current = null;
    }
  }

  function holdThenIdle(ackFace: MascotFace, ms = ACK_FACE_HOLD_MS) {
    clearAckTimer();
    setFace(ackFace);
    ackTimerRef.current = window.setTimeout(() => {
      ackTimerRef.current = null;
      setFace('idle');
    }, ms);
  }

  useEffect(() => {
    const unsub = subscribeChatEvents({
      onInferenceStart: () => {
        clearAckTimer();
        mascotLog('voice-session transition → thinking (inference_start)');
        setFace('thinking');
      },
      onIterationStart: e => {
        // Subsequent iterations mean the agent is grinding through tool rounds.
        if (e.round > 1) {
          clearAckTimer();
          mascotLog('voice-session transition → confused (iteration round=%d)', e.round);
          setFace('confused');
        }
      },
      onToolCall: () => {
        clearAckTimer();
        mascotLog('voice-session transition → confused (tool_call)');
        setFace('confused');
      },
      onToolResult: e => {
        if (!e.success) {
          mascotLog('voice-session transition → concerned (tool_result failed)');
          // Don't fully derail — let the next inference step take over.
          setFace('concerned');
        } else {
          setFace('thinking');
        }
      },
      onTextDelta: e => {
        // Pseudo-lipsync only kicks in if no real audio is playing.
        if (listeningRef.current) {
          mascotLog('voice-session text_delta suppressed — listening is active');
          return;
        }
        if (playbackRef.current) return;
        clearAckTimer();
        setFace('speaking');
        targetRef.current = pickViseme(e.delta);
        lastDeltaAtRef.current = window.performance.now();
      },
      onDone: e => {
        if (listeningRef.current) {
          mascotLog('voice-session onDone suppressed — listening is active');
          return;
        }
        const ackFace = pickConversationAckFace(e) ?? 'happy';
        if (!speakRef.current || !e.full_response?.trim()) {
          // Soft acknowledgement beat instead of snapping back to idle.
          holdThenIdle(ackFace);
          return;
        }
        // Fire-and-forget — startTtsPlayback owns its cleanup via finally.
        void startTtsPlayback(e.full_response, ackFace).catch(() => {});
      },
      onError: () => {
        mascotLog('voice-session transition → concerned (chat_error), cancelling in-flight TTS');
        // Bump seq to invalidate any in-flight startTtsPlayback awaiters.
        playbackSeqRef.current++;
        const orphan = playbackRef.current;
        playbackRef.current = null;
        if (orphan) {
          orphan.stop();
          // We're early-returning instead of awaiting `orphan.ended`, so the
          // stop()-sentinel rejection has no handler — attach one explicitly
          // or it surfaces as an unhandledrejection in Sentry (#1472).
          orphan.ended.catch(swallowAudioStop);
        }
        visemeFramesRef.current = [];
        holdThenIdle('concerned');
      },
    });
    return () => {
      unsub();
      clearAckTimer();
      // Same — invalidate in-flight callbacks before tearing down.
      playbackSeqRef.current++;
      const orphan = playbackRef.current;
      playbackRef.current = null;
      if (orphan) {
        orphan.stop();
        orphan.ended.catch(swallowAudioStop);
      }
    };
  }, []);

  useEffect(() => {
    if (!listening) return;
    clearAckTimer();
    mascotLog(
      'voice-session listening interrupt — cancelling TTS (had playback=%s)',
      !!playbackRef.current
    );
    // Treat mic-hot as an explicit interruption: stale synthesis/playback
    // callbacks must not switch the mascot back to speaking after we listen.
    playbackSeqRef.current++;
    const orphan = playbackRef.current;
    playbackRef.current = null;
    if (orphan) {
      orphan.stop();
      orphan.ended.catch(swallowAudioStop);
    }
    visemeFramesRef.current = [];
    visemeCursorRef.current = 0;
    targetRef.current = VISEMES.REST;
    lastDeltaAtRef.current = 0;
    setFace('idle');
  }, [listening]);

  async function startTtsPlayback(
    text: string,
    ackFace: ConversationAckFace = 'happy'
  ): Promise<void> {
    // Cancel any in-flight playback so its handle.ended callback can't reset
    // state belonging to the new run.
    const prev = playbackRef.current;
    playbackRef.current = null;
    if (prev) {
      prev.stop();
      prev.ended.catch(swallowAudioStop);
    }
    visemeFramesRef.current = [];
    visemeCursorRef.current = 0;
    clearAckTimer();
    const seq = ++playbackSeqRef.current;
    const isStillCurrent = () => playbackSeqRef.current === seq;
    let degraded = false;

    try {
      setFace('thinking');
      let tts;
      try {
        // Always pass the effective voice id — the selector already
        // resolves manual override / locale default / build-time
        // fallback to a single string, so `synthesizeSpeech` doesn't
        // need its own fallback branch here.
        tts = await synthesizeSpeech(text, { voiceId: mascotVoiceIdRef.current });
      } catch (err) {
        // Voice path unavailable — degrade cleanly to text-only behavior.
        if (isStillCurrent()) degraded = true;
        throw err;
      }
      if (!isStillCurrent()) return;
      let frames: VisemeFrame[] = tts.visemes ?? [];
      let source: 'visemes' | 'alignment' | 'procedural' = 'visemes';
      if (frames.length > 0 && !framesProduceMotion(frames)) {
        // Backend shipped frames but every code maps to REST — usually means
        // the codes are in a vocabulary `oculusVisemeToShape` doesn't know.
        // Drop them and let the alignment / procedural path take over so the
        // mouth doesn't sit on the rest-smile path for the whole clip.
        mascotLog('tts visemes produced no motion — dropping and falling through');
        frames = [];
      }
      if (frames.length === 0 && tts.alignment && tts.alignment.length > 0) {
        // Backend didn't ship viseme cues — derive a coarse track from char timings
        // so the mouth still animates in sync with the audio.
        frames = visemesFromAlignment(tts.alignment);
        source = 'alignment';
        mascotLog('tts derived %d viseme frames from alignment', frames.length);
      } else if (frames.length > 0) {
        mascotLog('tts got %d viseme frames from backend', frames.length);
      }
      // Start audio first — `playBase64Audio` calls `audio.play()` directly so
      // the user-gesture chain that authorized speech stays intact. If we
      // awaited anything else between the user click and play(), CEF would
      // reject playback under its autoplay policy.
      const handle = await playBase64Audio(tts.audio_base64, tts.audio_mime ?? 'audio/mpeg');
      if (!isStillCurrent()) {
        handle.stop();
        handle.ended.catch(swallowAudioStop);
        return;
      }
      if (frames.length === 0) {
        // Last-resort fallback: backend shipped neither viseme cues nor
        // alignment (e.g. the new public `tts-v1` model on the hosted
        // backend). Use whatever duration the decoder has reported so far —
        // `proceduralVisemes` falls back to a text-length estimate when the
        // metadata hasn't loaded yet, so we don't await it on the critical
        // path (waiting opens a window where audio plays under a static face).
        const dur = handle.durationMs();
        frames = proceduralVisemes(text, dur);
        source = 'procedural';
        mascotLog('tts derived %d procedural viseme frames over %dms', frames.length, dur);
      }
      visemeFramesRef.current = frames;
      visemeCursorRef.current = 0;
      playbackRef.current = handle;
      setFace('speaking');
      mascotLog(
        'tts playback started (%s) — driving lipsync from %d frames',
        source,
        frames.length
      );
      try {
        await handle.ended;
      } catch (err) {
        // Stop sentinel is expected when a newer turn cancels playback —
        // rethrow anything else so real decoder errors aren't masked.
        swallowAudioStop(err);
      }
    } catch (err) {
      if (isStillCurrent()) degraded = true;
      throw err;
    } finally {
      if (isStillCurrent()) {
        playbackRef.current = null;
        visemeFramesRef.current = [];
        if (degraded) {
          holdThenIdle('concerned');
        } else {
          holdThenIdle(ackFace);
        }
      }
    }
  }

  // RAF loop while we're speaking. TTS playback always sets face to
  // 'speaking' before awaiting the audio, so this also covers the audio-driven
  // viseme path.
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

  // `listening` is an external override so callers wiring dictation state
  // can reflect mic-on without racing the chat event subscription.
  const effectiveFace: MascotFace = listening ? 'listening' : face;
  const effectiveViseme: VisemeShape = listening ? VISEMES.REST : viseme;

  return { face: effectiveFace, viseme: effectiveViseme };
}
