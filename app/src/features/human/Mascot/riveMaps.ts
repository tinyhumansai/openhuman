/**
 * Pure mapping tables + helpers for the Rive mascot asset (`tiny_mascot.riv`).
 *
 * Kept free of any `@rive-app/*` import so the logic is cheap to unit-test and
 * can be shared without dragging the WebGL runtime into a test bundle.
 *
 * Asset contract (artboard `Artboard`, state machine `MascotSM`, view model
 * `ViewModel1`) — discovered by enumerating the `.riv` with the Rive runtime:
 *   - `pose`            enum `poses`        — drives the body animation
 *   - `mouthVisemeCode` enum `visme_codes`  — drives the mouth shape
 *   - `primaryColor` / `secondaryColor` colors
 */
import type { MascotFace } from './Ghosty';

/** State machine name baked into the asset. The `useRive` hook plays this. */
export const MASCOT_STATE_MACHINE = 'MascotSM';

/**
 * Every pose value the asset's `poses` enum accepts. Setting `pose` to a string
 * outside this set is a no-op in the state machine, so callers should only ever
 * emit values from here.
 */
export const RIVE_POSES = [
  'idle',
  'thinking',
  'celebration',
  'bookreading',
  'coffeedrink',
  'writing',
  'bobbateadrink',
  'recording',
  'hand_wave',
  'dancing',
] as const;
export type RivePose = (typeof RIVE_POSES)[number];

/**
 * Maps every {@link MascotFace} to the closest pose animation in the asset.
 */
export const FACE_TO_POSE: Record<MascotFace, RivePose> = {
  idle: 'idle',
  normal: 'idle',
  sleep: 'idle',
  listening: 'idle',
  thinking: 'thinking',
  confused: 'thinking',
  speaking: 'idle',
  happy: 'idle',
  concerned: 'thinking',
  curious: 'bookreading',
  proud: 'celebration',
  cautious: 'thinking',
  celebrating: 'celebration',
  writing: 'writing',
  reading: 'bookreading',
  recording: 'recording',
  waving: 'hand_wave',
  dancing: 'dancing',
  drinking_coffee: 'coffeedrink',
  drinking_boba: 'bobbateadrink',
};

export function faceToPose(face: MascotFace): RivePose {
  return FACE_TO_POSE[face] ?? 'idle';
}

/**
 * The exact 15 tokens the asset's `visme_codes` enum accepts — the standard
 * Oculus/ElevenLabs viseme set. The enum is case-sensitive, so the canonical
 * casing here is what must reach the state machine.
 */
export const RIVE_VISEME_SET = [
  'sil',
  'PP',
  'FF',
  'TH',
  'DD',
  'kk',
  'CH',
  'SS',
  'nn',
  'RR',
  'aa',
  'E',
  'ih',
  'oh',
  'ou',
] as const;
export type RiveVisemeCode = (typeof RIVE_VISEME_SET)[number];

/**
 * Lowercased-alias → canonical `visme_codes` token.
 *
 * Different TTS providers (and the text-delta pseudo-lipsync) disagree on
 * casing and on the close vowels: Oculus ships `I`/`O`/`U` where the Rive
 * asset names them `ih`/`oh`/`ou`. We normalise both so the mouth never
 * freezes, and fall back to `sil` for anything unrecognised — setting an
 * out-of-set enum string would otherwise be a silent no-op.
 */
const VISEME_ALIAS: Record<string, RiveVisemeCode> = {
  sil: 'sil',
  silence: 'sil',
  // Bilabials — fully closed
  pp: 'PP',
  m: 'PP',
  b: 'PP',
  p: 'PP',
  // Labiodentals
  ff: 'FF',
  f: 'FF',
  v: 'FF',
  // Dental / "th"
  th: 'TH',
  // Alveolar / velar plosives
  dd: 'DD',
  d: 'DD',
  t: 'DD',
  l: 'DD',
  kk: 'kk',
  k: 'kk',
  g: 'kk',
  // Affricate
  ch: 'CH',
  // Sibilants
  ss: 'SS',
  s: 'SS',
  z: 'SS',
  // Nasal
  nn: 'nn',
  n: 'nn',
  // Liquid r
  rr: 'RR',
  r: 'RR',
  // Open vowel
  aa: 'aa',
  a: 'aa',
  // Front mid vowel — the asset has a distinct `E`
  e: 'E',
  // Close-front → ih
  ih: 'ih',
  i: 'ih',
  y: 'ih',
  // Close-back rounded → oh / ou
  oh: 'oh',
  o: 'oh',
  ou: 'ou',
  u: 'ou',
  w: 'ou',
};

/**
 * Normalise any incoming viseme code (Oculus 15-set, bare letters, mixed
 * casing) to the exact `visme_codes` token the asset expects. Unknown codes
 * resolve to `sil` (mouth closed).
 */
export function toRiveVisemeCode(code: string): RiveVisemeCode {
  return VISEME_ALIAS[code.toLowerCase()] ?? 'sil';
}

/**
 * Poses the mascot drifts through on its own while otherwise idle, to keep it
 * feeling alive. Deliberately excludes `idle` (the resting state it returns to
 * between picks) and `recording` (reads as an active screen-capture cue).
 */
export const AMBIENT_POSES: readonly RivePose[] = [
  'thinking',
  'bookreading',
  'coffeedrink',
  'writing',
  'bobbateadrink',
  'dancing',
  'hand_wave',
  'celebration',
];

/**
 * Pick a random ambient pose, optionally avoiding `exclude` so the same pose
 * doesn't fire twice in a row. `rng` is injectable for deterministic tests.
 */
export function pickAmbientPose(exclude?: RivePose, rng: () => number = Math.random): RivePose {
  const pool = exclude ? AMBIENT_POSES.filter(p => p !== exclude) : AMBIENT_POSES;
  const choices = pool.length > 0 ? pool : AMBIENT_POSES;
  const idx = Math.min(choices.length - 1, Math.floor(rng() * choices.length));
  return choices[idx];
}
