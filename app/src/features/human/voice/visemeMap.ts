/**
 * Map ElevenLabs / Oculus 15-set visemes onto the mascot's mouth shapes.
 * The 15-set: sil, PP, FF, TH, DD, kk, CH, SS, nn, RR, aa, E, I, O, U.
 */
import { VISEMES, type VisemeShape } from '../Mascot/visemes';

const TABLE: Record<string, VisemeShape> = {
  sil: VISEMES.REST,
  // Bilabials — fully closed
  PP: VISEMES.M,
  // Labiodentals — lower lip tucked
  FF: VISEMES.F,
  // Dental, alveolar, velar — slight opening, modest width
  TH: { openness: 0.25, width: 0.5 },
  DD: { openness: 0.25, width: 0.5 },
  kk: { openness: 0.3, width: 0.5 },
  // Affricates / sibilants — narrow, slight opening
  CH: { openness: 0.2, width: 0.4 },
  SS: { openness: 0.18, width: 0.55 },
  // Nasal alveolar
  nn: { openness: 0.2, width: 0.45 },
  // Liquid r — rounded, mid
  RR: { openness: 0.35, width: 0.3 },
  // Vowels
  aa: VISEMES.A,
  E: VISEMES.E,
  I: VISEMES.I,
  O: VISEMES.O,
  U: VISEMES.U,
};

export function oculusVisemeToShape(viseme: string): VisemeShape {
  return TABLE[viseme] ?? VISEMES.REST;
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
