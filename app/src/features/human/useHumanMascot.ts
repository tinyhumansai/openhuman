import debug from 'debug';
import { useEffect, useRef, useState } from 'react';
import { useSelector } from 'react-redux';

import { type ChatSubagentDoneEvent, subscribeChatEvents } from '../../services/chatService';
import { selectEffectiveMascotVoiceId } from '../../store/mascotSlice';
import type { MascotFace } from './Mascot';
import { lerpViseme, VISEMES, type VisemeShape } from './Mascot/visemes';
import {
  isAudioStopped,
  type PlaybackHandle,
  playBase64Audio,
  swallowAudioStop,
} from './voice/audioPlayer';
import {
  hasUsableStarts,
  normalizeVisemeTimeline,
  proceduralVisemes,
  synthesizeSpeech,
  type VisemeFrame,
} from './voice/ttsClient';
import { selectVisemeSource, splitIntoSentences } from './voice/ttsQueue';
import { findActiveFrame, oculusVisemeToShape } from './voice/visemeMap';

const mascotLog = debug('human:mascot');

/** ms the mouth holds the target viseme before decaying back to rest. */
const VISEME_DECAY_MS = 180;

/**
 * Safety ceiling for a single TTS utterance. Runaway audio (network stall,
 * decoder that never emits `ended`) is auto-stopped at this limit so the
 * mascot never gets stuck in a permanent `speaking` pose.
 * 5 minutes comfortably covers any real reply; exported for tests.
 */
export const TTS_MAX_PLAYBACK_MS = 5 * 60 * 1_000;
const TTS_ESTIMATED_MS_PER_CHAR = 55;
const TTS_MIN_ESTIMATED_PLAYBACK_MS = 600;

function estimateTtsPlaybackMs(text: string, frames: VisemeFrame[]): number {
  const frameEnd = frames.at(-1)?.end_ms ?? 0;
  if (frameEnd > 0 && hasUsableStarts(frames)) return frameEnd;
  return Math.max(TTS_MIN_ESTIMATED_PLAYBACK_MS, text.trim().length * TTS_ESTIMATED_MS_PER_CHAR);
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

/**
 * Pick a raw Oculus viseme code from a text delta character. Used to drive
 * `mouthVisemeCode` on the Rive state machine during pseudo-lipsync (no TTS).
 * Returns Oculus 15-set codes; `RiveMascot` normalises the close vowels
 * (`I`→`ih`, `O`→`oh`, `U`→`ou`) to the asset's `visme_codes` vocabulary at
 * render time, keeping `E`/`aa` and the consonants as-is.
 */
export function pickVisemeCode(delta: string): string {
  const ch = delta
    .replace(/[^a-zA-Z]/g, '')
    .slice(-1)
    .toLowerCase();
  switch (ch) {
    case 'a':
      return 'aa';
    case 'e':
      return 'E';
    case 'i':
    case 'y':
      return 'I';
    case 'o':
      return 'O';
    case 'u':
    case 'w':
      return 'U';
    case 'm':
    case 'b':
    case 'p':
      return 'PP';
    case 'f':
    case 'v':
      return 'FF';
    case 's':
    case 'z':
      return 'SS';
    case 'n':
    case 'l':
      return 'nn';
    case 't':
    case 'd':
      return 'DD';
    case 'k':
    case 'g':
      return 'kk';
    case 'r':
      return 'RR';
    default:
      return 'E';
  }
}

type ConversationAckFace = Extract<
  MascotFace,
  | 'happy'
  | 'confused'
  | 'concerned'
  | 'curious'
  | 'proud'
  | 'cautious'
  | 'celebrating'
  | 'dancing'
  | 'waving'
>;
type ConversationAckEvent = { full_response?: string | null; reaction_emoji?: string | null };

const HAPPY_REACTION_EMOJIS = new Set(['✅', '🎉', '🙌', '😊', '😄', '👍', '💪']);
const PROUD_REACTION_EMOJIS = new Set(['⭐', '🌟', '🏆', '🎯', '💯', '🚀', '✨', '🥇']);
const CURIOUS_REACTION_EMOJIS = new Set(['🔍', '💭', '🧐', '🤓', '👀']);
const CONFUSED_REACTION_EMOJIS = new Set(['🤔', '❓', '❔']);
const CAUTIOUS_REACTION_EMOJIS = new Set(['⚠️', '⚠', '💡', '⚡']);
const CONCERNED_REACTION_EMOJIS = new Set(['🚨', '❌', '😕', '😟']);
const CELEBRATING_REACTION_EMOJIS = new Set(['🥳', '🍾', '🎊', '🎈', '🪅']);
const DANCING_REACTION_EMOJIS = new Set(['💃', '🕺', '🎵', '🎶', '🎸']);
const WAVING_REACTION_EMOJIS = new Set(['👋', '🤝', '🫡']);

const CONCERNED_TEXT_RE =
  /\b(sorry|apolog(?:y|ize|ise)|failed|failure|error|cannot|can't|unable|blocked|problem)\b/i;
const CONFUSED_TEXT_RE =
  /\b(not sure|unclear|ambiguous|clarify|which one|need more|can you confirm|maybe)\b/i;
const HAPPY_TEXT_RE = /\b(done|completed|fixed|success|successful|ready|all set|great|nice)\b/i;
const PROUD_TEXT_RE =
  /\b(successfully completed|all tasks? (done|finished)|mission accomplished|everything (works?|is working)|all (checks?|tests?) pass(ed)?)\b/i;
const CURIOUS_TEXT_RE =
  /\b(interesting|fascinating|curious(ly)?|let me (check|look|investigate)|i('ll)? (look|check) into|turns? out to be)\b/i;
const CAUTIOUS_TEXT_RE =
  /\b(be careful|warning|caution|heads? up|please note|make sure|important to note|note that|worth (noting|mentioning))\b/i;
const CELEBRATING_TEXT_RE =
  /\b(congrat(ulations|s)?|well done|bravo|hooray|woohoo|amazing|fantastic|incredible|awesome work)\b/i;
const GREETING_TEXT_RE =
  /^(hello|hey|hi there|good (morning|afternoon|evening)|welcome back|greetings|howdy)[!.,]?(?:\s|$)/i;

/**
 * Map conversation-level meaning into the short acknowledgement face that
 * follows a completed turn. Runtime activity still owns thinking/speaking
 * states; this only decides the post-turn emotional beat.
 */
export function pickConversationAckFace(event: ConversationAckEvent): ConversationAckFace | null {
  const reaction = event.reaction_emoji?.trim();
  if (reaction) {
    if (CELEBRATING_REACTION_EMOJIS.has(reaction)) return 'celebrating';
    if (DANCING_REACTION_EMOJIS.has(reaction)) return 'dancing';
    if (WAVING_REACTION_EMOJIS.has(reaction)) return 'waving';
    if (PROUD_REACTION_EMOJIS.has(reaction)) return 'proud';
    if (HAPPY_REACTION_EMOJIS.has(reaction)) return 'happy';
    if (CURIOUS_REACTION_EMOJIS.has(reaction)) return 'curious';
    if (CONFUSED_REACTION_EMOJIS.has(reaction)) return 'confused';
    if (CAUTIOUS_REACTION_EMOJIS.has(reaction)) return 'cautious';
    if (CONCERNED_REACTION_EMOJIS.has(reaction)) return 'concerned';
  }

  const text = event.full_response?.trim() ?? '';
  if (!text) return null;
  // Priority: concerned > cautious > proud > confused > curious > happy.
  // Concerned and cautious share some vocabulary; check concerned first so
  // outright failures don't get softened to a heads-up.
  if (CONCERNED_TEXT_RE.test(text)) return 'concerned';
  if (CAUTIOUS_TEXT_RE.test(text)) return 'cautious';
  if (CELEBRATING_TEXT_RE.test(text)) return 'celebrating';
  if (PROUD_TEXT_RE.test(text)) return 'proud';
  if (CONFUSED_TEXT_RE.test(text)) return 'confused';
  if (CURIOUS_TEXT_RE.test(text)) return 'curious';
  if (GREETING_TEXT_RE.test(text)) return 'waving';
  if (HAPPY_TEXT_RE.test(text)) return 'happy';
  return null;
}

/**
 * Map a tool name to an activity pose. Returns null when the tool doesn't
 * have a strong visual association — the caller falls back to a generic face.
 */
function toolToActivityFace(toolName: string): MascotFace | null {
  const name = toolName.toLowerCase();

  if (
    name.includes('file_write') ||
    name.includes('edit_file') ||
    name.includes('apply_patch') ||
    name.includes('create_file') ||
    name.includes('write')
  ) {
    return 'writing';
  }

  if (
    name.includes('browser') ||
    name.includes('web_search') ||
    name.includes('web_fetch') ||
    name.includes('read_file') ||
    name.includes('search') ||
    name.includes('grep') ||
    name.includes('find')
  ) {
    return 'reading';
  }

  return null;
}

interface UseHumanMascotOptions {
  /** When true, post-stream replies are sent to ElevenLabs and the mouth
   *  follows the returned viseme timeline while the audio plays. */
  speakReplies?: boolean;
  /** When true, force the mascot into a `listening` pose. Caller is responsible
   *  for setting this while the mic is hot (e.g. from voice dictation state). */
  listening?: boolean;
}

interface UseHumanMascotResult {
  face: MascotFace;
  viseme: VisemeShape;
  /** Raw Oculus 15-set viseme code for Rive's `mouthVisemeCode` input. */
  visemeCode: string;
}

/** Result of a chunk's speech synthesis — `ok:false` on failure so the pump
 *  can skip a bad sentence without aborting the whole reply. */
type ChunkSynth = { ok: true; tts: Awaited<ReturnType<typeof synthesizeSpeech>> } | { ok: false };

/** One queued sentence: its spoken text plus the eagerly-fired synth promise. */
interface TtsChunk {
  text: string;
  synth: Promise<ChunkSynth>;
}

/**
 * Drives the mascot's face/mouth from agent + voice lifecycle events.
 *
 * Mapping (kept in one place so the visual model stays coherent):
 *
 * - `inference_start` → `thinking`
 * - `iteration_start` round > 1 or `tool_call` → activity pose based on tool
 *   name (writing/reading) or `confused` as fallback
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

  const effectiveMascotVoiceId = useSelector(selectEffectiveMascotVoiceId);
  const mascotVoiceIdRef = useRef<string>(effectiveMascotVoiceId);
  mascotVoiceIdRef.current = effectiveMascotVoiceId;

  const [face, setFace] = useState<MascotFace>('idle');
  const targetRef = useRef<VisemeShape>(VISEMES.REST);
  const visemeCodeRef = useRef<string>('sil');
  const lastDeltaAtRef = useRef(0);
  const ackTimerRef = useRef<number | null>(null);

  const toolSucceededRef = useRef(false);
  const subagentSucceededRef = useRef(false);

  const playbackRef = useRef<PlaybackHandle | null>(null);
  const visemeFramesRef = useRef<{ viseme: string; start_ms: number; end_ms: number }[]>([]);
  const visemeCursorRef = useRef(0);
  const playbackSeqRef = useRef(0);
  // Wall-clock anchor (performance.now() at the instant playback became
  // current) used to index the viseme timeline. We deliberately do NOT key off
  // `audio.currentTime`: in the embedded CEF webview it can fail to advance for
  // in-memory blob audio, which freezes the mouth on a single viseme even
  // though the audio plays. A monotonic clock always advances, and because the
  // viseme frames are rescaled to the measured audio duration it stays in sync.
  const playbackStartedAtRef = useRef(0);
  // Throttle marker for the lipsync diagnostic log (last logged ms).
  const lastLipsyncLogRef = useRef(0);

  // ── Streaming, sentence-chunked TTS state (#5358) ──────────────────────────
  // The reply is spoken sentence-by-sentence as it streams instead of waiting
  // for the whole response, so the first audio starts within a sentence's
  // synth latency rather than after the entire reply + one big round trip.
  //
  // `turnSeqRef` mirrors the `playbackSeqRef` value captured when the current
  // TTS turn began — every async synth/play continuation re-checks it so a
  // superseded or cancelled turn drops its work instead of speaking over the
  // next one. A chunk holds the raw sentence text plus the in-flight synthesis
  // promise (fired eagerly at enqueue so sentence N+1 synthesizes while sentence
  // N plays); the pump plays them strictly in enqueue order, single-flight, so
  // only one `<audio>` element is ever live and there are no orphan blob URLs.
  const streamActiveRef = useRef(false);
  const turnSeqRef = useRef(0);
  const sawDeltaThisTurnRef = useRef(false);
  const sentenceBufferRef = useRef('');
  const chunkQueueRef = useRef<TtsChunk[]>([]);
  const queueClosedRef = useRef(false);
  const queuePumpingRef = useRef(false);
  const queueAckRef = useRef<ConversationAckFace>('happy');
  const chunkAttemptedRef = useRef(false);
  const chunkPlayedRef = useRef(false);

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
        toolSucceededRef.current = false;
        subagentSucceededRef.current = false;
        // A new agent turn supersedes any reply still being spoken — stop the
        // previous turn's audio and drop its queue so the next turn's deltas
        // start a fresh stream instead of appending onto stale state.
        cancelTtsPlayback();
        mascotLog('voice-session transition → thinking (inference_start)');
        setFace('thinking');
      },
      onIterationStart: e => {
        if (e.round > 1) {
          clearAckTimer();
          mascotLog('voice-session transition → drinking_coffee (iteration round=%d)', e.round);
          setFace('drinking_coffee');
        }
      },
      onToolCall: e => {
        clearAckTimer();
        const activityFace = toolToActivityFace(e.tool_name);
        if (activityFace) {
          mascotLog('voice-session transition → %s (tool_call %s)', activityFace, e.tool_name);
          setFace(activityFace);
        } else {
          mascotLog('voice-session transition → thinking (tool_call %s)', e.tool_name);
          setFace('thinking');
        }
      },
      onToolResult: e => {
        if (!e.success) {
          mascotLog('voice-session transition → concerned (tool_result failed)');
          setFace('concerned');
        } else {
          toolSucceededRef.current = true;
          setFace('thinking');
        }
      },
      onSubagentDone: (e: ChatSubagentDoneEvent) => {
        if (e.success) {
          mascotLog('voice-session subagent_done success tool=%s', e.tool_name);
          subagentSucceededRef.current = true;
        } else {
          mascotLog(
            'voice-session transition → concerned (subagent_done failed tool=%s)',
            e.tool_name
          );
          setFace('concerned');
        }
      },
      onTextDelta: e => {
        if (listeningRef.current) {
          mascotLog('voice-session text_delta suppressed — listening is active');
          return;
        }
        // In speak mode we stream the reply sentence-by-sentence: buffer the
        // deltas and enqueue each complete sentence for synthesis + playback the
        // moment it lands, so the first audio starts within one sentence's
        // synth latency instead of after the whole reply + one big round trip
        // (#5358). The text-delta pseudo-lipsync below is the no-audio path
        // only — letting it run while replies are spoken would flap the mouth
        // ahead of the voice.
        if (speakRef.current) {
          ensureTtsTurn();
          sawDeltaThisTurnRef.current = true;
          enqueueSpeech(e.delta, turnSeqRef.current, false);
          return;
        }
        if (playbackRef.current) return;
        clearAckTimer();
        setFace('speaking');
        targetRef.current = pickViseme(e.delta);
        visemeCodeRef.current = pickVisemeCode(e.delta);
        lastDeltaAtRef.current = window.performance.now();
      },
      onDone: e => {
        if (listeningRef.current) {
          mascotLog('voice-session onDone suppressed — listening is active');
          return;
        }
        const didMeaningfulWork = toolSucceededRef.current || subagentSucceededRef.current;
        const explicitAck = pickConversationAckFace(e);
        const ackFace: ConversationAckFace =
          (explicitAck === 'happy' || explicitAck === null) && didMeaningfulWork
            ? 'celebrating'
            : (explicitAck ?? 'happy');
        toolSucceededRef.current = false;
        subagentSucceededRef.current = false;
        mascotLog(
          'voice-session onDone ackFace=%s (explicit=%s didWork=%s)',
          ackFace,
          explicitAck ?? 'none',
          didMeaningfulWork
        );
        if (!speakRef.current || !e.full_response?.trim()) {
          holdThenIdle(ackFace);
          return;
        }
        finalizeTtsTurn(e.full_response, ackFace);
      },
      onError: () => {
        mascotLog('voice-session transition → concerned (chat_error), cancelling in-flight TTS');
        cancelTtsPlayback();
        holdThenIdle('concerned');
      },
    });
    return () => {
      unsub();
      clearAckTimer();
      cancelTtsPlayback();
    };
  }, []);

  useEffect(() => {
    if (!listening) return;
    clearAckTimer();
    const ttsWasInFlight = playbackRef.current != null || streamActiveRef.current;
    mascotLog(
      'voice-session listening-active tts-in-flight=%s — %s',
      ttsWasInFlight,
      ttsWasInFlight
        ? 'user started recording while TTS was playing (interrupted)'
        : 'mic activated, no TTS to cancel'
    );
    // Barge-in: drain the whole streaming queue (active clip + any pending
    // sentences), not just the current handle, so a mid-reply interruption
    // stops everything at once.
    cancelTtsPlayback();
    targetRef.current = VISEMES.REST;
    lastDeltaAtRef.current = 0;
    setFace('idle');
  }, [listening]);

  function isTurnCurrent(seq: number): boolean {
    return playbackSeqRef.current === seq;
  }

  /**
   * Reset all playback + queue state and begin a fresh TTS turn. Cancels any
   * clip still playing from a prior turn and bumps the seq token so its
   * in-flight synth/play continuations no-op. Returns the new turn's seq.
   */
  function beginTtsTurn(): number {
    const orphan = playbackRef.current;
    playbackRef.current = null;
    playbackStartedAtRef.current = 0;
    if (orphan) {
      orphan.stop();
      orphan.ended.catch(swallowAudioStop);
    }
    const seq = ++playbackSeqRef.current;
    turnSeqRef.current = seq;
    streamActiveRef.current = true;
    sawDeltaThisTurnRef.current = false;
    sentenceBufferRef.current = '';
    chunkQueueRef.current = [];
    queueClosedRef.current = false;
    queuePumpingRef.current = false;
    chunkAttemptedRef.current = false;
    chunkPlayedRef.current = false;
    queueAckRef.current = 'happy';
    visemeFramesRef.current = [];
    visemeCursorRef.current = 0;
    visemeCodeRef.current = 'sil';
    lastLipsyncLogRef.current = -1_000;
    clearAckTimer();
    setFace('thinking');
    mascotLog('tts turn %d started', seq);
    return seq;
  }

  /**
   * Start a streaming turn on the first delta of a spoken reply. No-op if one
   * is already open (subsequent deltas of the same reply).
   */
  function ensureTtsTurn(): void {
    // Reuse the open turn only while it is still accepting sentences. Once
    // `chat_done` has closed it (queueClosedRef), a delta belongs to a NEW turn
    // — start fresh so it never appends onto the previous reply's queue/ack.
    if (streamActiveRef.current && !queueClosedRef.current) return;
    beginTtsTurn();
  }

  /**
   * Cancel everything: the active clip, any queued sentences, and the buffer.
   * Bumps the seq so pending synth/play continuations drop their work; leaves
   * the face for the caller to set. Idempotent.
   */
  function cancelTtsPlayback(): void {
    playbackSeqRef.current++;
    streamActiveRef.current = false;
    queueClosedRef.current = false;
    queuePumpingRef.current = false;
    sawDeltaThisTurnRef.current = false;
    sentenceBufferRef.current = '';
    chunkQueueRef.current = [];
    const orphan = playbackRef.current;
    playbackRef.current = null;
    playbackStartedAtRef.current = 0;
    if (orphan) {
      orphan.stop();
      orphan.ended.catch(swallowAudioStop);
    }
    visemeFramesRef.current = [];
    visemeCursorRef.current = 0;
    visemeCodeRef.current = 'sil';
  }

  /**
   * Buffer streamed text, carve out complete sentences, and enqueue each for
   * synthesis. `flush` also enqueues the trailing partial sentence (used at
   * end-of-turn).
   */
  function enqueueSpeech(text: string, seq: number, flush: boolean): void {
    if (!isTurnCurrent(seq)) return;
    sentenceBufferRef.current += text;
    const { sentences, rest } = splitIntoSentences(sentenceBufferRef.current);
    sentenceBufferRef.current = rest;
    for (const sentence of sentences) enqueueSentence(sentence, seq);
    if (flush) {
      const tail = sentenceBufferRef.current.trim();
      sentenceBufferRef.current = '';
      if (tail) enqueueSentence(tail, seq);
    }
  }

  /** Fire a sentence's synthesis eagerly and queue it for in-order playback. */
  function enqueueSentence(sentence: string, seq: number): void {
    const spoken = sentence.trim();
    if (!spoken) return;
    chunkAttemptedRef.current = true;
    const synth: Promise<ChunkSynth> = synthesizeSpeech(spoken, {
      voiceId: mascotVoiceIdRef.current,
    })
      .then(tts => ({ ok: true, tts }) as ChunkSynth)
      .catch(() => ({ ok: false }) as ChunkSynth);
    chunkQueueRef.current.push({ text: spoken, synth });
    mascotLog(
      'tts enqueued sentence chars=%d queue=%d',
      spoken.length,
      chunkQueueRef.current.length
    );
    void pumpQueue(seq);
  }

  /**
   * Finalize the turn on `chat_done`. If the reply streamed as deltas, flush
   * the trailing fragment and close the queue; otherwise (no deltas were seen)
   * speak the whole response as a fresh turn.
   */
  function finalizeTtsTurn(fullResponse: string, ackFace: ConversationAckFace): void {
    if (sawDeltaThisTurnRef.current && streamActiveRef.current) {
      queueAckRef.current = ackFace;
      const seq = turnSeqRef.current;
      enqueueSpeech('', seq, true);
      queueClosedRef.current = true;
      void pumpQueue(seq);
      return;
    }
    // No deltas arrived (some backends only emit the final message) — start a
    // fresh turn and speak the whole response, still chunked into sentences.
    const seq = beginTtsTurn();
    queueAckRef.current = ackFace;
    enqueueSpeech(fullResponse, seq, true);
    queueClosedRef.current = true;
    void pumpQueue(seq);
  }

  /**
   * Play queued chunks strictly in order, one at a time. Runs until the queue
   * drains; a later enqueue restarts it. Finishes the turn once the queue is
   * drained and closed.
   */
  async function pumpQueue(seq: number): Promise<void> {
    if (queuePumpingRef.current) return;
    if (!isTurnCurrent(seq)) return;
    queuePumpingRef.current = true;
    try {
      while (chunkQueueRef.current.length > 0) {
        if (!isTurnCurrent(seq)) return;
        const chunk = chunkQueueRef.current.shift()!;
        const result = await chunk.synth;
        if (!isTurnCurrent(seq)) return;
        if (!result.ok) {
          mascotLog('tts chunk synth failed — skipping');
          continue;
        }
        await playChunk(result.tts, chunk.text, seq);
      }
    } finally {
      // Only act if this pump still owns the turn — a superseding turn resets
      // state explicitly, so a stale pump must not touch it. Finalizing here in
      // the `finally` guarantees the turn always ends (mouth rests, ack fires)
      // even if a chunk unexpectedly throws, instead of stranding it speaking.
      if (isTurnCurrent(seq)) {
        queuePumpingRef.current = false;
        maybeFinishTurn(seq);
      }
    }
  }

  /**
   * Play one chunk: derive its own viseme timeline, play the audio, drive the
   * mouth off it, and resolve when it ends. Keeps `face='speaking'` across
   * chunk boundaries so the RAF loop never tears down between sentences.
   */
  async function playChunk(
    tts: Awaited<ReturnType<typeof synthesizeSpeech>>,
    text: string,
    seq: number
  ): Promise<void> {
    let { frames, source } = selectVisemeSource(tts);
    let handle: PlaybackHandle;
    try {
      handle = await playBase64Audio(tts.audio_base64, tts.audio_mime ?? 'audio/mpeg', {
        maxDurationMs: TTS_MAX_PLAYBACK_MS,
      });
    } catch (err) {
      // A decode/autoplay failure degrades this chunk; the turn ends concerned
      // only if no chunk ever played (see maybeFinishTurn).
      mascotLog('tts chunk playback could not start: %s', String(err));
      return;
    }
    if (!isTurnCurrent(seq)) {
      handle.stop();
      handle.ended.catch(swallowAudioStop);
      return;
    }

    // Start lipsync as soon as play() succeeds; metadata can lag by the 500ms
    // decoder fallback, so estimate first and refine once it resolves.
    let audioMs = handle.durationMs();
    const waitingForMetadata = audioMs <= 0;
    if (waitingForMetadata) audioMs = estimateTtsPlaybackMs(text, frames);
    if (frames.length === 0) {
      frames = proceduralVisemes(text, audioMs);
      source = 'procedural';
    }
    frames = normalizeVisemeTimeline(frames, audioMs);

    visemeFramesRef.current = frames;
    visemeCursorRef.current = 0;
    playbackRef.current = handle;
    playbackStartedAtRef.current = window.performance.now();
    lastLipsyncLogRef.current = -1_000;
    chunkPlayedRef.current = true;
    setFace('speaking');
    mascotLog('tts chunk playback started (%s) frames=%d', source, frames.length);

    if (waitingForMetadata) {
      await handle.metadataReady;
      if (!isTurnCurrent(seq)) {
        handle.stop();
        handle.ended.catch(swallowAudioStop);
        return;
      }
      const measuredAudioMs = handle.durationMs();
      if (measuredAudioMs > 0 && playbackRef.current === handle) {
        const measuredFrames =
          source === 'procedural'
            ? proceduralVisemes(text, measuredAudioMs)
            : normalizeVisemeTimeline(visemeFramesRef.current, measuredAudioMs);
        visemeFramesRef.current =
          source === 'procedural'
            ? normalizeVisemeTimeline(measuredFrames, measuredAudioMs)
            : measuredFrames;
        visemeCursorRef.current = 0;
      }
    }

    try {
      await handle.ended;
    } catch (err) {
      // A real decoder/playback error (not the stop sentinel) degrades this
      // chunk. Do NOT rethrow — that would escape the pump, skip the cleanup
      // below, and strand the mascot in the speaking state. Log and fall
      // through so the handle is released and the queue finishes normally.
      if (!isAudioStopped(err)) {
        mascotLog('tts chunk ended with a playback error: %s', String(err));
      }
    }
    // Release the finished clip so the between-chunk gap rests the mouth. The
    // face stays 'speaking'; the next chunk or the finish transition follows.
    if (isTurnCurrent(seq) && playbackRef.current === handle) {
      playbackRef.current = null;
      playbackStartedAtRef.current = 0;
    }
  }

  /**
   * Once the queue is drained and closed, end the turn: rest the mouth and play
   * the acknowledgement beat — or a concerned beat if a sentence was attempted
   * but nothing ever spoke (total synth/playback failure).
   */
  function maybeFinishTurn(seq: number): void {
    if (!isTurnCurrent(seq)) return;
    if (!streamActiveRef.current) return;
    if (!queueClosedRef.current) return;
    if (chunkQueueRef.current.length > 0) return;
    if (queuePumpingRef.current) return;
    streamActiveRef.current = false;
    playbackRef.current = null;
    playbackStartedAtRef.current = 0;
    visemeFramesRef.current = [];
    visemeCodeRef.current = 'sil';
    const degraded = chunkAttemptedRef.current && !chunkPlayedRef.current;
    mascotLog('tts turn %d finished degraded=%s', seq, degraded);
    holdThenIdle(degraded ? 'concerned' : queueAckRef.current);
  }

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
  let visemeCode = 'sil';
  const playback = playbackRef.current;
  if (playback) {
    const audioMs = playback.currentMs();
    const wallClockMs =
      playbackStartedAtRef.current > 0
        ? window.performance.now() - playbackStartedAtRef.current
        : audioMs;
    const ms = audioMs < 0 ? -1 : Math.max(audioMs, wallClockMs);
    if (ms >= 0) {
      const { frame, cursor } = findActiveFrame(
        visemeFramesRef.current,
        ms,
        visemeCursorRef.current
      );
      visemeCursorRef.current = cursor;
      viseme = frame ? oculusVisemeToShape(frame.viseme) : VISEMES.REST;
      visemeCode = frame ? frame.viseme : 'sil';
      if (ms - lastLipsyncLogRef.current >= 500) {
        lastLipsyncLogRef.current = ms;
        mascotLog(
          'lipsync ms=%d cursor=%d/%d code=%s',
          Math.round(ms),
          cursor,
          visemeFramesRef.current.length,
          visemeCode
        );
      }
    }
  } else if (face === 'speaking') {
    const since = window.performance.now() - lastDeltaAtRef.current;
    const decay = Math.max(0, Math.min(1, since / VISEME_DECAY_MS));
    viseme = lerpViseme(targetRef.current, VISEMES.REST, decay);
    visemeCode = decay > 0.5 ? 'sil' : visemeCodeRef.current;
  }

  const effectiveFace: MascotFace = listening ? 'listening' : face;
  const effectiveViseme: VisemeShape = listening ? VISEMES.REST : viseme;
  const effectiveVisemeCode: string = listening ? 'sil' : visemeCode;

  return { face: effectiveFace, viseme: effectiveViseme, visemeCode: effectiveVisemeCode };
}
