/**
 * Map ElevenLabs / Oculus 15-set visemes onto the mascot's mouth shapes.
 * The 15-set: sil, PP, FF, TH, DD, kk, CH, SS, nn, RR, aa, E, I, O, U.
 */
import { VISEMES, type VisemeShape } from '../Mascot/visemes';

/**
 * Lookup keyed by lowercased viseme code so the table tolerates whatever
 * casing the backend ships (`PP` / `pp`, `aa` / `Aa`, etc). Different TTS
 * providers — and even different ElevenLabs models — disagree on casing,
 * and a single-case table silently maps every frame to REST, leaving the
 * mascot's mouth frozen on the rest-smile path while audio plays.
 */
const TABLE: Record<string, VisemeShape> = {
  sil: VISEMES.REST,
  silence: VISEMES.REST,
  // Bilabials — fully closed
  pp: VISEMES.M,
  m: VISEMES.M,
  b: VISEMES.M,
  p: VISEMES.M,
  // Labiodentals — lower lip tucked
  ff: VISEMES.F,
  f: VISEMES.F,
  v: VISEMES.F,
  // Dental, alveolar, velar — slight opening, modest width
  th: { openness: 0.25, width: 0.5 },
  dd: { openness: 0.25, width: 0.5 },
  d: { openness: 0.25, width: 0.5 },
  t: { openness: 0.25, width: 0.5 },
  l: { openness: 0.25, width: 0.5 },
  kk: { openness: 0.3, width: 0.5 },
  k: { openness: 0.3, width: 0.5 },
  g: { openness: 0.3, width: 0.5 },
  // Affricates / sibilants — narrow, slight opening
  ch: { openness: 0.2, width: 0.4 },
  ss: { openness: 0.18, width: 0.55 },
  s: { openness: 0.18, width: 0.55 },
  z: { openness: 0.18, width: 0.55 },
  // Nasal alveolar
  nn: { openness: 0.2, width: 0.45 },
  n: { openness: 0.2, width: 0.45 },
  // Liquid r — rounded, mid
  rr: { openness: 0.35, width: 0.3 },
  r: { openness: 0.35, width: 0.3 },
  // Vowels — accept both 15-set codes (`aa`, `E`, …) and bare letters.
  aa: VISEMES.A,
  a: VISEMES.A,
  e: VISEMES.E,
  i: VISEMES.I,
  y: VISEMES.I,
  o: VISEMES.O,
  u: VISEMES.U,
  w: VISEMES.U,
};

export function oculusVisemeToShape(viseme: string): VisemeShape {
  return TABLE[viseme.toLowerCase()] ?? VISEMES.REST;
}

export interface TimedFrame {
  viseme: string;
  start_ms: number;
  end_ms: number;
}

/**
 * Find the active viseme frame at `ms` using a sticky cursor — viseme tracks
 * are monotonic, so we resume from the last hit instead of re-scanning. Pass
 * the previous return as `cursor` on the next call.
 */
export function findActiveFrame(
  frames: TimedFrame[],
  ms: number,
  cursor = 0
): { frame: TimedFrame | null; cursor: number } {
  if (frames.length === 0) return { frame: null, cursor: 0 };
  let i = Math.max(0, Math.min(cursor, frames.length - 1));
  // Rewind if the caller jumped backward (e.g. replay).
  while (i > 0 && frames[i].start_ms > ms) i--;
  while (i < frames.length - 1 && frames[i].end_ms <= ms) i++;
  const f = frames[i];
  if (ms >= f.start_ms && ms <= f.end_ms) return { frame: f, cursor: i };
  return { frame: null, cursor: i };
}
