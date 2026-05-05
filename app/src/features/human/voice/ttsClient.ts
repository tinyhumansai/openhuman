import debug from 'debug';

import { callCoreRpc } from '../../../services/coreRpcClient';
import { MASCOT_VOICE_ID } from '../../../utils/config';

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
  const spoken = prepareForSpeech(text);
  const params: Record<string, unknown> = { text: spoken };
  if (voiceId) params.voice_id = voiceId;
  if (opts.modelId) params.model_id = opts.modelId;
  if (opts.outputFormat) params.output_format = opts.outputFormat;
  ttsLog(
    'synthesize chars=%d (raw=%d) voice=%s',
    spoken.length,
    text.length,
    voiceId ?? 'default'
  );

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
