/**
 * Mouth shape primitives for the mascot. A viseme is a `{openness, width}`
 * pair; the renderer turns it into an SVG path centered on the mouth area.
 *
 * The resting "smile" is special-cased — when openness collapses to 0 we draw
 * the original happy curve so the idle face stays alive.
 */

export type VisemeId = 'REST' | 'A' | 'E' | 'I' | 'O' | 'U' | 'M' | 'F';

export interface VisemeShape {
  /** 0 = closed, 1 = fully open vertically. */
  openness: number;
  /** 0 = pursed (O/U), 1 = wide (E/I). */
  width: number;
}

export const VISEMES: Record<VisemeId, VisemeShape> = {
  REST: { openness: 0, width: 0.3 },
  A: { openness: 0.95, width: 0.6 },
  E: { openness: 0.45, width: 1.0 },
  I: { openness: 0.3, width: 0.85 },
  O: { openness: 0.75, width: 0.2 },
  U: { openness: 0.4, width: 0.05 },
  M: { openness: 0, width: 0.4 },
  F: { openness: 0.15, width: 0.55 },
};

export const REST_SMILE_PATH = 'M478,570 Q520,617 562,570 Q520,597 478,570 Z';

/** Linear interpolation between two viseme shapes. */
export function lerpViseme(a: VisemeShape, b: VisemeShape, t: number): VisemeShape {
  const k = Math.max(0, Math.min(1, t));
  return {
    openness: a.openness + (b.openness - a.openness) * k,
    width: a.width + (b.width - a.width) * k,
  };
}

/** Anchor point for the mouth oval. */
const CX = 520;
const CY = 590;

/**
 * Build the SVG `d` attribute for a mouth shape. When `openness` is near zero
 * we fall back to the resting smile so the idle face doesn't look slack.
 */
export function visemePath(shape: VisemeShape): string {
  if (shape.openness < 0.05) return REST_SMILE_PATH;
  const halfW = 22 + shape.width * 28;
  const halfH = 4 + shape.openness * 28;
  const left = CX - halfW;
  const right = CX + halfW;
  const top = CY - halfH;
  const bot = CY + halfH;
  return `M${left},${CY} Q${CX},${top} ${right},${CY} Q${CX},${bot} ${left},${CY} Z`;
}
