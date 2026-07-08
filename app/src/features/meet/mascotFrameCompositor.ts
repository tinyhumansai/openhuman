/**
 * Pure canvas-geometry helpers for the Meet video producer (issue #4277).
 *
 * Kept free of any WebGL / DOM-mount dependency so the containment math is
 * cheap to unit-test in isolation. The producer (`MascotFrameProducer.tsx`)
 * owns the OffscreenCanvas + WebSocket pipeline; this module owns only the
 * "where does each mascot get drawn" arithmetic, so the no-clip guarantee
 * (AC#6) is proven by the compositor tests rather than a WebGL render.
 */

/** Frame width for a single mascot (unchanged from the original producer). */
export const FRAME_W = 320;
/** Frame height for a single mascot (unchanged from the original producer). */
export const FRAME_H = 240;
/**
 * Frame dimensions when two mascots share the frame side-by-side (issue #4277).
 * Wider than the single frame so each mascot keeps roughly the single-frame
 * cell width instead of being squeezed to half.
 *
 * 480×270 is deliberately 16:9 — the same aspect as the fake-camera capture
 * canvas (1280×720). The Tauri camera bridge cover-scales the received frame
 * onto that canvas (`scale = Math.max(W/bw, H/bh)` in `camera_bridge.js`), so a
 * frame whose aspect differs from 16:9 gets its overflowing axis cropped. A
 * 480×240 (2:1) dual frame would be scaled to 1440×720 and lose ~27 source px
 * off each side — clipping the two mascots' outer edges. Matching 16:9 makes the
 * cover-scale a pure fit with no crop.
 */
export const FRAME_W_DUAL = 480;
export const FRAME_H_DUAL = 270;
/**
 * Fraction of each cell reserved as padding on every side before the mascot
 * is scaled to fit. Matches the original single-mascot inset so the framing
 * is visually identical in the single path.
 */
export const MASCOT_INSET = 0.06;

/**
 * Draw `sourceCanvas` scaled to *contain* (never crop) inside the cell at
 * `(cellX, cellY)` of size `cellW × cellH`, centred, with `inset` padding on
 * every side.
 *
 * Contain-scaling (`min` of the two axis ratios) guarantees the mascot always
 * fits within the padded cell, so it can never be clipped by the cell edge
 * (AC#6) regardless of the source canvas aspect ratio. The mascot is centred
 * within the cell so any leftover space is split evenly.
 *
 * Returns the computed destination rect for the caller's diagnostics (and so
 * the geometry is directly assertable in tests) — the draw itself is the side
 * effect.
 */
export function drawMascotInCell(
  ctx: {
    drawImage: (image: CanvasImageSource, dx: number, dy: number, dw: number, dh: number) => void;
  },
  sourceCanvas: CanvasImageSource,
  cellX: number,
  cellY: number,
  cellW: number,
  cellH: number,
  inset: number,
  srcW: number,
  srcH: number
): { dx: number; dy: number; dw: number; dh: number } {
  // Guard against a zero-sized source (a canvas that hasn't laid out yet):
  // scaling by it would produce NaN and a silent no-op draw.
  const safeSrcW = srcW > 0 ? srcW : 1;
  const safeSrcH = srcH > 0 ? srcH : 1;

  const fitW = cellW * (1 - 2 * inset);
  const fitH = cellH * (1 - 2 * inset);
  const scale = Math.min(fitW / safeSrcW, fitH / safeSrcH);
  const dw = safeSrcW * scale;
  const dh = safeSrcH * scale;
  const dx = cellX + (cellW - dw) / 2;
  const dy = cellY + (cellH - dh) / 2;
  ctx.drawImage(sourceCanvas, dx, dy, dw, dh);
  return { dx, dy, dw, dh };
}

/**
 * Sample a coarse 7×5 grid of luma values from a rendered frame. Used as a
 * cheap "is the mascot actually on the frame or is it black?" diagnostic that
 * the producer streams over the debug WebSocket every couple of seconds.
 *
 * Moved here from `MascotFrameProducer.tsx` (issue #4277) so it lives next to
 * the rest of the frame geometry; the producer re-exports it for back-compat
 * with existing importers.
 */
export function sampleCanvasPixels(
  ctx: OffscreenCanvasRenderingContext2D,
  width: number,
  height: number
) {
  const cols = 7;
  const rows = 5;
  let sum = 0;
  let min = 255;
  let max = 0;
  let count = 0;
  let dark = 0;
  let bright = 0;

  try {
    for (let y = 0; y < rows; y++) {
      for (let x = 0; x < cols; x++) {
        const px = Math.max(0, Math.min(width - 1, Math.floor(((x + 0.5) * width) / cols)));
        const py = Math.max(0, Math.min(height - 1, Math.floor(((y + 0.5) * height) / rows)));
        const [r, g, b] = ctx.getImageData(px, py, 1, 1).data;
        const luma = Math.round(r * 0.299 + g * 0.587 + b * 0.114);
        sum += luma;
        min = Math.min(min, luma);
        max = Math.max(max, luma);
        if (luma < 8) dark++;
        if (luma > 32) bright++;
        count++;
      }
    }
    return {
      avgLuma: Math.round(sum / Math.max(1, count)),
      minLuma: min,
      maxLuma: max,
      darkSamples: dark,
      brightSamples: bright,
      sampleCount: count,
    };
  } catch (err) {
    return { error: String(err instanceof Error ? err.message : err) };
  }
}
