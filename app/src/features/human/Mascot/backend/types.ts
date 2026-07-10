// Mascot library types — mirror the backend models in
// `tinyhumansai/backend:src/services/mascots/types.ts` and
// `src/database/models/mascot.ts`. Kept in TS-only form here because
// the app talks to the backend over HTTP — there is no shared package.

export type MascotVariableType = 'color' | 'number' | 'string';

export interface MascotVariable {
  name: string;
  label: string;
  type: MascotVariableType;
  default: string | number;
  description?: string;
}

export type MascotTweenKind = 'translateY' | 'rotate' | 'blink';

/**
 * Declarative per-frame tween entry. Interpreted by the renderer's animation
 * loop — there is no per-mascot animation code, just data. `kind` selects
 * the math; the remaining fields parametrize it.
 *
 *   translateY  transform="translate(0  amp*sin(2π·freq·t + phase))"
 *   rotate      transform="rotate(amp*sin(2π·freq·t + phase)  pivotX pivotY)"
 *   blink       y-axis squish: pulses to `closed` for `duration`s every
 *               `period`s, otherwise scale 1
 */
export interface MascotTween {
  /** Element id (bare, no '#') to mutate on the mounted SVG. */
  id: string;
  kind: MascotTweenKind;
  /** translateY/rotate: cycles per second. */
  freq?: number;
  /** translateY: pixels. rotate: degrees. */
  amp?: number;
  /** Phase offset in seconds. */
  phase?: number;
  /** rotate/blink: SVG-space pivot point [x, y]. */
  pivot?: [number, number];
  /** blink: full cycle in seconds. */
  period?: number;
  /** blink: how long the eye stays scrunched, in seconds. */
  duration?: number;
  /** blink: scaleY value while closed (default 0.12). */
  closed?: number;
}

export interface MascotState {
  id: string;
  label: string;
  description: string;
  /** Full <svg>...</svg> for this state, served by the backend per-id endpoint. */
  svg: string;
  /** Optional rAF-driven tween config. Empty/absent => static pose. */
  tween?: MascotTween[];
}

export interface MascotViseme {
  label: string;
  description: string;
  svg: string;
}

export type MascotFormat = 'svg' | 'rive';

export interface MascotSummary {
  id: string;
  name: string;
  version: string;
  description: string;
  /** 'svg' (legacy) or 'rive' (current). Absent on old backends => treat as svg. */
  format?: MascotFormat;
  /** SVG-only: state metadata (no SVG bytes or tween) to keep list payload light. */
  states?: Pick<MascotState, 'id' | 'label' | 'description'>[];
  hasVisemes?: boolean;
  /** Rive-only: maps logical state ids to Rive pose enum values. */
  stateToPose?: Record<string, string>;
}

export interface MascotDetail {
  id: string;
  name: string;
  version: string;
  description: string;
  format?: 'svg';
  viewBox: string;
  defaultState: string;
  variables: MascotVariable[];
  states: MascotState[];
  visemes: MascotViseme[];
  visemeSlot?: string;
  hidesOnViseme?: string[];
}

export type ViewModelInputType = 'number' | 'boolean' | 'color' | 'string' | 'enum';

export interface ViewModelInput {
  name: string;
  type: ViewModelInputType;
  description?: string;
}

/** Rive mascot manifest — the binary lives at `rivFileUrl` (version-stamped). */
export interface RiveMascotDetail {
  id: string;
  name: string;
  version: string;
  description: string;
  format: 'rive';
  /** Backend-relative URL of the .riv binary, e.g. `/mascots/toshi/riv?v=1.0.0`. */
  rivFileUrl: string;
  source?: 'builtin' | 'custom';
  defaultState: string;
  stateToPose: Record<string, string>;
  viewModelInputs: ViewModelInput[];
}

export type MascotDetailUnion = MascotDetail | RiveMascotDetail;

export function isRiveMascotDetail(d: MascotDetailUnion): d is RiveMascotDetail {
  return d.format === 'rive';
}

/** Wire shapes for /mascots and /mascots/:id. */
export interface ListMascotsResponse {
  success: true;
  data: { mascots: MascotSummary[] };
}

export interface GetMascotResponse {
  success: true;
  data: { mascot: MascotDetailUnion };
}
