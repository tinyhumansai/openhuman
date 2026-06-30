import debug from 'debug';

import { callCoreRpc } from '../../../services/coreRpcClient';
import { MASCOT_VOICE_ID, MASCOT_VOICE_MODEL_ID } from '../../../utils/config';

const ttsLog = debug('human:tts');

/**
 * One frame on the viseme timeline. Backend emits the Oculus / Microsoft
 * 15-set: `sil, PP, FF, TH, DD, kk, CH, SS, nn, RR, aa, E, I, O, U`.
 */
export interface VisemeFrame {
  viseme: string;
  start_ms: number;
  end_ms: number;
}

export interface AlignmentFrame {
  char: string;
  start_ms: number;
  end_ms: number;
}

/**
 * Normalized response from the core RPC `openhuman.voice_reply_synthesize`.
 * The core does the messy "tolerate multiple backend response shapes" work
 * (see `src/openhuman/voice/reply_speech.rs`) so the UI can stay strict.
 */
export interface TtsResponse {
  audio_base64: string;
  audio_mime: string;
  visemes: VisemeFrame[];
  alignment?: AlignmentFrame[];
}

export interface TtsOptions {
  voiceId?: string;
  modelId?: string;
  outputFormat?: string;
}

/**
 * Synthesize agent reply speech via the Rust core. The core proxies the
 * hosted backend's `/openai/v1/audio/speech` endpoint so the WebView never
 * touches it directly, which sidesteps a class of "Load failed" CORS/TLS
 * issues and keeps auth in one place.
 */
export async function synthesizeSpeech(text: string, opts: TtsOptions = {}): Promise<TtsResponse> {
  const voiceId = opts.voiceId ?? MASCOT_VOICE_ID;
  // Default model is `eleven_multilingual_v2` so non-English locales
  // render their native script instead of being phoneticised by an
  // English-only model. Callers can still override via `modelId`.
  const modelId = opts.modelId ?? MASCOT_VOICE_MODEL_ID;
  // `prepareForSpeech` collapses to '' on replies that are pure code/markdown
  // formatting. The core RPC rejects empty text, which would propagate as a
  // visible error for what was effectively a no-op reply. Fall back to the
  // raw trimmed text, then to a single ellipsis (so the mascot just exhales)
  // before letting an empty payload reach the upstream.
  const spoken = prepareForSpeech(text) || text.trim() || '...';
  const params: Record<string, unknown> = { text: spoken };
  if (voiceId) params.voice_id = voiceId;
  if (modelId) params.model_id = modelId;
  if (opts.outputFormat) params.output_format = opts.outputFormat;
  ttsLog('synthesize chars=%d (raw=%d) voice=%s', spoken.length, text.length, voiceId ?? 'default');

  const result = await callCoreRpc<TtsResponse>({
    method: 'openhuman.voice_reply_synthesize',
    params,
  });

  ttsLog(
    'synthesize done audio_bytes=%d visemes=%d alignment=%d',
    result.audio_base64.length,
    result.visemes.length,
    result.alignment?.length ?? 0
  );
  return result;
}

/**
 * Fall back to deriving rough visemes from char-level alignment if the backend
 * didn't return them. Uses the same heuristic as text-stream pseudo-lipsync —
 * picks a mouth shape from the last letter in each ~80ms window. Kept on the
 * client so it can run after the audio arrives without an extra round trip.
 */
export function visemesFromAlignment(alignment: AlignmentFrame[]): VisemeFrame[] {
  if (alignment.length === 0) return [];
  const WINDOW_MS = 80;
  const out: VisemeFrame[] = [];
  let bucketStart = alignment[0].start_ms;
  let bucketEnd = bucketStart + WINDOW_MS;
  let bucketChars = '';
  for (const a of alignment) {
    if (a.start_ms >= bucketEnd) {
      if (bucketChars.length > 0) {
        out.push({
          viseme: alignmentLetterToCode(bucketChars),
          start_ms: bucketStart,
          end_ms: bucketEnd,
        });
      }
      bucketStart = a.start_ms;
      bucketEnd = bucketStart + WINDOW_MS;
      bucketChars = '';
    }
    bucketChars += a.char;
  }
  if (bucketChars.length > 0) {
    out.push({
      viseme: alignmentLetterToCode(bucketChars),
      start_ms: bucketStart,
      end_ms: bucketEnd,
    });
  }
  return out;
}

/**
 * Does this viseme track carry a genuinely usable per-frame start timeline?
 *
 * The cloud TTS path sometimes ships visemes with all-zero (or otherwise
 * collapsed) `start_ms`, which can't represent the gaps between words. We treat
 * the timing as usable only when there are at least two frames, the furthest
 * start is non-zero, and most starts are distinct (a real, spread-out timeline).
 */
export function hasUsableStarts(frames: VisemeFrame[]): boolean {
  if (frames.length < 2) return false;
  const maxStart = Math.max(...frames.map(f => f.start_ms));
  if (!(maxStart > 0)) return false;
  const distinct = new Set(frames.map(f => f.start_ms)).size;
  return distinct > frames.length * 0.5;
}

/**
 * Turn a backend viseme list into a walkable timeline matched to the audio.
 *
 * The cloud TTS path ships viseme *codes* in order but with unreliable
 * timestamps — observed shapes include a constant `end_ms` (= whole-utterance
 * length) on every frame, and all-zero `start_ms`. Either way the naive
 * `findActiveFrame` freezes the mouth (frame 0 "covers" everything, or every
 * frame is zero-length so it snaps to the last `sil`).
 *
 * Strategy:
 *  - If the frames carry a genuinely usable timeline — strictly-ish increasing
 *    starts that span most of the audio — keep it, just rebuilding each end from
 *    the next start (visemes are cue points) and stretching the last to the end.
 *  - Otherwise distribute the viseme *sequence* evenly across the measured audio
 *    duration. We lose the natural per-phoneme rhythm but keep the real phonetic
 *    order, so the mouth animates start-to-finish in lockstep with the voice.
 */
export function normalizeVisemeTimeline(frames: VisemeFrame[], totalMs: number): VisemeFrame[] {
  if (frames.length === 0) return frames;
  const sorted = [...frames].sort((a, b) => a.start_ms - b.start_ms);
  const maxStart = sorted[sorted.length - 1].start_ms;
  const total = totalMs > 0 ? totalMs : Math.max(maxStart, frames.length * 80);

  // A real timeline's last cue starts deep into the clip. If the furthest start
  // is in the front portion (e.g. every start is 0), the timestamps are junk —
  // fall back to even distribution.
  const hasUsableTimeline = maxStart > total * 0.5;
  if (hasUsableTimeline) {
    // Preserve the real per-frame end. A gap between one frame's end and the
    // next frame's start is a genuine pause — findActiveFrame returns no frame
    // there, so the mouth rests (sil) instead of holding a shape across the
    // silence. We only clamp ends so a frame never overruns the next cue.
    return sorted.map((f, i) => {
      const next = sorted[i + 1];
      const end = next ? Math.min(f.end_ms, next.start_ms) : Math.max(f.end_ms, total);
      return { viseme: f.viseme, start_ms: f.start_ms, end_ms: Math.max(f.start_ms, end) };
    });
  }

  const step = total / frames.length;
  return sorted.map((f, i) => ({
    viseme: f.viseme,
    start_ms: Math.round(i * step),
    end_ms: Math.round((i + 1) * step),
  }));
}

/**
 * Reshape an assistant message into something the TTS engine can read with
 * natural cadence. The agent's reply is markdown — raw `**bold**`, headings,
 * code fences, link syntax, and `\n\n` paragraph breaks all confuse
 * ElevenLabs' prosody model and collapse the pauses between sentences. We
 * strip the formatting and translate paragraph boundaries into an explicit
 * `...` pause, which ElevenLabs honors as a beat between thoughts.
 *
 * Exported for tests so the mapping can be pinned without going through the
 * full RPC stack.
 */
export function prepareForSpeech(raw: string): string {
  let s = raw ?? '';
  // Drop fenced code blocks entirely — reading symbols out loud is painful and
  // they almost never carry the intent of the reply.
  s = s.replace(/```[\s\S]*?```/g, ' ');
  // Inline code → keep the contents, drop the backticks.
  s = s.replace(/`([^`]+)`/g, '$1');
  // Markdown links `[label](url)` → just the label.
  s = s.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '$1');
  // Bare URLs read terribly — replace with a short stand-in.
  s = s.replace(/https?:\/\/\S+/g, 'a link');
  // Headings, blockquotes, list bullets at line start.
  s = s.replace(/^\s{0,3}#{1,6}\s+/gm, '');
  s = s.replace(/^\s{0,3}>\s?/gm, '');
  s = s.replace(/^\s*[-*+]\s+/gm, '');
  s = s.replace(/^\s*\d+\.\s+/gm, '');
  // Emphasis markers — keep the words, drop the wrappers.
  s = s.replace(/(\*\*|__)(.*?)\1/g, '$2');
  s = s.replace(/(\*|_)(.*?)\1/g, '$2');
  // Convert paragraph breaks into an explicit ellipsis pause before we collapse
  // whitespace, otherwise the double newline becomes a single space.
  s = s.replace(/\n{2,}/g, ' ... ');
  // Single newlines inside a paragraph are just soft wraps in markdown.
  s = s.replace(/\n+/g, ' ');
  // Ensure a sentence terminator at the very end so the voice doesn't trail
  // upward like an unfinished thought.
  s = s.trim();
  if (s.length > 0 && !/[.!?…]$/.test(s)) s += '.';
  // Collapse any runs of whitespace introduced by the substitutions above.
  s = s.replace(/[ \t]{2,}/g, ' ');
  return s;
}

function alignmentLetterToCode(chunk: string): string {
  const ch = chunk.replace(/[^a-zA-Z]/g, '').slice(-1);
  return letterToOculusViseme(ch);
}

function letterToOculusViseme(ch: string): string {
  switch (ch.toLowerCase()) {
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
    case 'r':
      return 'RR';
    case 'n':
      return 'nn';
    case 'l':
    case 'd':
    case 't':
      return 'DD';
    case 'k':
    case 'g':
      return 'kk';
    case 'h':
    case 'c':
    case 'j':
      return 'CH';
    default:
      return 'sil';
  }
}

/**
 * Last-resort fallback when the backend returns neither viseme cues nor
 * char-level alignment (e.g. when the TTS provider / model strips timing
 * data). Walks the source text and distributes visemes evenly across the
 * known audio duration so the mouth still animates in lockstep with audio
 * playback instead of freezing on REST.
 *
 * Spaces collapse to `sil` so word boundaries read as natural pauses.
 * Per-frame duration is clamped to [60ms, 160ms] — fast enough that the
 * mouth doesn't feel slack on long replies, slow enough to stay readable
 * on short ones.
 */
export function proceduralVisemes(text: string, durationMs: number): VisemeFrame[] {
  const cleaned = text.replace(/\s+/g, ' ').trim();
  if (cleaned.length === 0) return [];
  const total = durationMs > 0 && Number.isFinite(durationMs) ? durationMs : cleaned.length * 80;
  const step = Math.max(60, Math.min(160, total / cleaned.length));
  const frames: VisemeFrame[] = [];
  let t = 0;
  for (const ch of cleaned) {
    const code = ch === ' ' ? 'sil' : letterToOculusViseme(ch);
    const start = Math.round(t);
    const end = Math.round(t + step);
    if (end > start) {
      frames.push({ viseme: code, start_ms: start, end_ms: end });
    }
    t += step;
  }
  return frames;
}
