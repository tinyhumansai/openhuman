/**
 * Pure helpers for streaming, sentence-chunked TTS playback.
 *
 * The Human tab used to wait for the *entire* agent reply to finish streaming
 * before synthesizing a single audio clip, which left a 3–4s gap between the
 * text appearing and the voice starting (#5358). Instead we now speak the reply
 * sentence by sentence as it streams: `splitIntoSentences` carves complete
 * sentences out of the growing delta buffer so each can be synthesized and
 * queued the moment it is ready, and `selectVisemeSource` picks the best
 * per-chunk viseme timeline for lip-sync.
 *
 * These helpers are deliberately framework-free (no React) so they can be unit
 * tested in isolation; the queue/pump orchestration that drives them lives in
 * `useHumanMascot`.
 */
import { VISEMES } from '../Mascot/visemes';
import { hasUsableStarts, type VisemeFrame, visemesFromAlignment } from './ttsClient';
import { oculusVisemeToShape } from './visemeMap';

/**
 * Sentence-ending punctuation immediately followed by whitespace. The
 * whitespace lookahead is what keeps decimals (`3.14`) and version numbers
 * (`v1.2`) from being treated as sentence breaks — their `.` is followed by a
 * digit, not a space. Abbreviations like `e.g. ` still split, but a stray short
 * chunk only costs an extra beat, never a crash.
 */
const SENTENCE_BOUNDARY_RE = /[.!?…]+(?=\s)/g;

/**
 * Carve every *complete* sentence out of the accumulated delta buffer, leaving
 * the trailing incomplete fragment as `rest` for the next delta to extend.
 *
 * A sentence is a run ending in `.`/`!`/`?`/`…` followed by whitespace. The
 * terminator stays attached to the sentence; the whitespace after it is dropped
 * (both the sentence and the rest are consumers of trimmed text).
 */
export function splitIntoSentences(buffer: string): { sentences: string[]; rest: string } {
  const sentences: string[] = [];
  let lastIndex = 0;
  SENTENCE_BOUNDARY_RE.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = SENTENCE_BOUNDARY_RE.exec(buffer)) !== null) {
    const end = match.index + match[0].length;
    const sentence = buffer.slice(lastIndex, end).trim();
    if (sentence) sentences.push(sentence);
    lastIndex = end;
  }
  return { sentences, rest: buffer.slice(lastIndex) };
}

/**
 * Does this timeline contain at least one frame whose code maps to a non-REST
 * mouth shape? Detects the "backend shipped frames in an unknown vocabulary"
 * regression where every viseme falls back to REST and the mouth visibly
 * freezes even though audio is playing.
 */
export function framesProduceMotion(frames: VisemeFrame[]): boolean {
  for (const frame of frames) {
    if (oculusVisemeToShape(frame.viseme) !== VISEMES.REST) return true;
  }
  return false;
}

export interface VisemeSelection {
  frames: VisemeFrame[];
  source: 'visemes' | 'alignment' | 'procedural';
}

interface SelectableTts {
  visemes?: VisemeFrame[];
  alignment?: { char: string; start_ms: number; end_ms: number }[];
}

/**
 * Choose the best per-chunk viseme timeline for a synthesized clip.
 *
 * Priority: backend viseme cues (if they actually produce motion and carry a
 * usable start timeline) → char-level alignment (which preserves the real
 * pauses between words when the viseme starts are degenerate) → an empty list,
 * signalling the caller to fall back to procedural visemes over the measured
 * audio duration.
 */
export function selectVisemeSource(tts: SelectableTts): VisemeSelection {
  let frames = tts.visemes ?? [];
  const source: VisemeSelection['source'] = 'visemes';
  // Drop frames that would every map to REST — an unknown-vocabulary track is
  // worse than none because it suppresses the procedural fallback.
  if (frames.length > 0 && !framesProduceMotion(frames)) frames = [];

  const startsUsable = hasUsableStarts(frames);
  const haveAlignment = !!tts.alignment && tts.alignment.length > 0;
  if ((frames.length === 0 || !startsUsable) && haveAlignment) {
    return { frames: visemesFromAlignment(tts.alignment!), source: 'alignment' };
  }
  return { frames, source };
}
