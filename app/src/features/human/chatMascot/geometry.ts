/**
 * Geometry for the chat mascot's dock ⇄ stage transition.
 *
 * The mascot is rendered **once**, in a fixed-position box that is always laid
 * out at a constant `STAGE_RENDER_PX` square and moved purely with a CSS
 * `transform`. Animating `width`/`height` instead would resize the Rive canvas
 * backing store on every frame; a transform is composited and never touches the
 * canvas, so the same single `.riv` instance serves both states.
 */

/**
 * Side length the mascot's canvas is actually rendered at, in CSS px.
 *
 * The mascot is rendered **once, big** — at roughly the size the old Human page
 * gave it (`min(80vh, 90%)`) — and then only ever scaled *down* onto whichever
 * anchor it is sitting on. That direction matters: a canvas is a texture, so
 * scaling it up is a raster stretch (the blur), while scaling it down
 * supersamples and stays crisp.
 *
 * It has to clear the display size by a real margin, not just match it:
 * `Fit.Contain` letterboxes the artwork inside the canvas, so the mascot itself
 * only occupies part of these pixels. Raising this makes the mascot sharper and
 * costs GPU (the canvas is re-rendered every frame during lipsync); lowering it
 * does the reverse.
 */
export const STAGE_RENDER_PX = 768;

/** Side length of the small mascot standing on the composer, in CSS px. */
export const DOCK_PX = 64;

/** Dock ⇄ stage travel time. Long enough to read as a move, short enough to feel instant. */
export const TRANSITION_MS = 320;

/**
 * A square anchor on screen. Only squares are ever interpolated — the dock and
 * the stage are both square, which is what lets a single uniform `scale` map
 * one onto the other without distorting the mascot.
 */
export interface MascotBox {
  left: number;
  top: number;
  /** Side length. Height is always equal. */
  size: number;
}

/**
 * The largest square that fits inside a measured element, centred within it.
 * The stage placeholder is `aspect-square` in practice, but a short window can
 * squash it — taking the inscribed square keeps the mascot undistorted instead
 * of letting the transform stretch it.
 */
export function inscribedSquare(rect: {
  left: number;
  top: number;
  width: number;
  height: number;
}): MascotBox {
  const size = Math.min(rect.width, rect.height);
  return {
    left: rect.left + (rect.width - size) / 2,
    top: rect.top + (rect.height - size) / 2,
    size,
  };
}

/** Standard ease-in-out. `t` is clamped to [0, 1]. */
export function easeInOutCubic(t: number): number {
  const x = Math.min(1, Math.max(0, t));
  return x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2;
}

/**
 * Round a box's position to whole CSS pixels.
 *
 * Applied to the *resting* state only. A canvas parked at a fractional offset
 * (`translate3d(40.37px, …)`) is resampled across the pixel grid by the
 * compositor, which reads as a soft, slightly blurry mascot even when the scale
 * is correct. Mid-travel the sub-pixel precision is what keeps the motion
 * smooth, so it is kept there and dropped the moment the mascot lands.
 */
export function snapBox(box: MascotBox): MascotBox {
  return { left: Math.round(box.left), top: Math.round(box.top), size: box.size };
}

/** Interpolate between two square anchors. `t = 0` → `from`, `t = 1` → `to`. */
export function lerpBox(from: MascotBox, to: MascotBox, t: number): MascotBox {
  return {
    left: from.left + (to.left - from.left) * t,
    top: from.top + (to.top - from.top) * t,
    size: from.size + (to.size - from.size) * t,
  };
}

/**
 * CSS `transform` that maps the constant `STAGE_RENDER_PX` render box (laid out
 * at viewport origin, `transform-origin: top left`) onto `box`.
 *
 * Guards a zero/negative render size so a mis-measured frame degrades to
 * "invisible" rather than emitting `scale(Infinity)`.
 */
export function boxTransform(box: MascotBox, renderPx: number = STAGE_RENDER_PX): string {
  const scale = renderPx > 0 ? box.size / renderPx : 0;
  return `translate3d(${box.left.toFixed(2)}px, ${box.top.toFixed(2)}px, 0) scale(${scale.toFixed(5)})`;
}

/** Whether the user has asked for reduced motion. Safe when `matchMedia` is absent. */
export function prefersReducedMotion(): boolean {
  try {
    return window.matchMedia?.('(prefers-reduced-motion: reduce)').matches === true;
  } catch {
    return false;
  }
}
