import { describe, expect, it, vi } from 'vitest';

import {
  drawMascotInCell,
  FRAME_H,
  FRAME_H_DUAL,
  FRAME_W,
  FRAME_W_DUAL,
  MASCOT_INSET,
  sampleCanvasPixels,
} from '../mascotFrameCompositor';

/** A drawImage-only ctx stub that records the last destination rect. */
function makeCtx() {
  const drawImage =
    vi.fn<(image: CanvasImageSource, dx: number, dy: number, dw: number, dh: number) => void>();
  return { drawImage };
}

describe('mascotFrameCompositor geometry constants', () => {
  it('exposes the locked single/dual frame geometry', () => {
    expect(FRAME_W).toBe(320);
    expect(FRAME_W_DUAL).toBe(480);
    expect(FRAME_H).toBe(240);
    expect(FRAME_H_DUAL).toBe(270);
    expect(MASCOT_INSET).toBeCloseTo(0.06);
  });

  it('keeps the dual frame at 16:9 so the camera bridge cover-scale never crops', () => {
    // The Tauri camera bridge cover-scales the frame onto a 1280×720 (16:9)
    // canvas; any non-16:9 frame loses its overflowing axis. Locking the dual
    // frame to 16:9 keeps both mascots' outer edges intact.
    expect(FRAME_W_DUAL / FRAME_H_DUAL).toBeCloseTo(16 / 9);
  });
});

describe('drawMascotInCell — containment (AC#6)', () => {
  it('contain-scales a TALL source and centers it (fits height, letterboxed x)', () => {
    const ctx = makeCtx();
    // Cell = full single frame; source is taller than wide.
    const srcW = 100;
    const srcH = 400;
    const rect = drawMascotInCell(
      ctx,
      {} as CanvasImageSource,
      0,
      0,
      FRAME_W,
      FRAME_H,
      MASCOT_INSET,
      srcW,
      srcH
    );

    const fitW = FRAME_W * (1 - 2 * MASCOT_INSET); // 281.6
    const fitH = FRAME_H * (1 - 2 * MASCOT_INSET); // 211.2
    const scale = Math.min(fitW / srcW, fitH / srcH); // height-bound → fitH/400
    const dw = srcW * scale;
    const dh = srcH * scale;
    const dx = (FRAME_W - dw) / 2;
    const dy = (FRAME_H - dh) / 2;

    // Height-bound: dh must equal the padded fit height, and never exceed the cell.
    expect(dh).toBeCloseTo(fitH);
    expect(rect.dw).toBeCloseTo(dw);
    expect(rect.dh).toBeCloseTo(dh);
    expect(rect.dx).toBeCloseTo(dx);
    expect(rect.dy).toBeCloseTo(dy);
    // No clipping: the drawn rect stays fully inside the cell bounds.
    expect(rect.dx).toBeGreaterThanOrEqual(0);
    expect(rect.dy).toBeGreaterThanOrEqual(0);
    expect(rect.dx + rect.dw).toBeLessThanOrEqual(FRAME_W + 1e-6);
    expect(rect.dy + rect.dh).toBeLessThanOrEqual(FRAME_H + 1e-6);
    expect(ctx.drawImage).toHaveBeenCalledWith(expect.anything(), dx, dy, dw, dh);
  });

  it('contain-scales a WIDE source and centers it (fits width, letterboxed y)', () => {
    const ctx = makeCtx();
    const srcW = 400;
    const srcH = 100;
    const rect = drawMascotInCell(
      ctx,
      {} as CanvasImageSource,
      0,
      0,
      FRAME_W,
      FRAME_H,
      MASCOT_INSET,
      srcW,
      srcH
    );

    const fitW = FRAME_W * (1 - 2 * MASCOT_INSET);
    const fitH = FRAME_H * (1 - 2 * MASCOT_INSET);
    const scale = Math.min(fitW / srcW, fitH / srcH); // width-bound → fitW/400
    const dw = srcW * scale;
    const dh = srcH * scale;

    // Width-bound: dw must equal the padded fit width.
    expect(dw).toBeCloseTo(fitW);
    expect(rect.dw).toBeCloseTo(dw);
    expect(rect.dh).toBeCloseTo(dh);
    // Still fully contained.
    expect(rect.dx).toBeGreaterThanOrEqual(0);
    expect(rect.dx + rect.dw).toBeLessThanOrEqual(FRAME_W + 1e-6);
    expect(rect.dy + rect.dh).toBeLessThanOrEqual(FRAME_H + 1e-6);
  });

  it('draws into each half-cell in dual mode without crossing the divider', () => {
    const half = FRAME_W_DUAL / 2; // 240
    // Square source so scale is symmetric and easy to reason about.
    const srcW = 200;
    const srcH = 200;

    const leftCtx = makeCtx();
    const left = drawMascotInCell(
      leftCtx,
      {} as CanvasImageSource,
      0,
      0,
      half,
      FRAME_H,
      MASCOT_INSET,
      srcW,
      srcH
    );
    const rightCtx = makeCtx();
    const right = drawMascotInCell(
      rightCtx,
      {} as CanvasImageSource,
      half,
      0,
      half,
      FRAME_H,
      MASCOT_INSET,
      srcW,
      srcH
    );

    // Left cell stays entirely left of the divider.
    expect(left.dx).toBeGreaterThanOrEqual(0);
    expect(left.dx + left.dw).toBeLessThanOrEqual(half + 1e-6);
    // Right cell stays entirely right of the divider.
    expect(right.dx).toBeGreaterThanOrEqual(half - 1e-6);
    expect(right.dx + right.dw).toBeLessThanOrEqual(FRAME_W_DUAL + 1e-6);
    // Both cells share the same size (identical source + cell dims).
    expect(right.dw).toBeCloseTo(left.dw);
    expect(right.dh).toBeCloseTo(left.dh);
    // The right cell is offset by exactly `half` from the left one.
    expect(right.dx - left.dx).toBeCloseTo(half);
  });

  it('never divides by zero for a not-yet-laid-out (0×0) source', () => {
    const ctx = makeCtx();
    const rect = drawMascotInCell(
      ctx,
      {} as CanvasImageSource,
      0,
      0,
      FRAME_W,
      FRAME_H,
      MASCOT_INSET,
      0,
      0
    );
    // With the 1px guard the rect is finite and contained, not NaN.
    expect(Number.isFinite(rect.dw)).toBe(true);
    expect(Number.isFinite(rect.dh)).toBe(true);
    expect(rect.dx).toBeGreaterThanOrEqual(0);
    expect(rect.dy).toBeGreaterThanOrEqual(0);
  });
});

// Moved from MascotFrameProducer.test.tsx (issue #4277) — sampleCanvasPixels
// now lives in the compositor module.
describe('sampleCanvasPixels', () => {
  it('returns pixel stats for a canvas with mid-range luma', () => {
    // luma = 0.299*128 + 0.587*128 + 0.114*128 ≈ 128
    const mockCtx = {
      getImageData: vi.fn().mockReturnValue({ data: [128, 128, 128, 255] }),
    } as unknown as OffscreenCanvasRenderingContext2D;

    const result = sampleCanvasPixels(mockCtx, 320, 240);
    expect(result).toMatchObject({
      avgLuma: 128,
      minLuma: 128,
      maxLuma: 128,
      darkSamples: 0,
      brightSamples: 35, // all 35 samples have luma > 32
      sampleCount: 35, // 7 cols × 5 rows
    });
  });

  it('counts dark samples correctly for near-black pixels', () => {
    // luma ≈ 0.299*4 + 0.587*4 + 0.114*4 ≈ 4 → dark (< 8), not bright (> 32)
    const mockCtx = {
      getImageData: vi.fn().mockReturnValue({ data: [4, 4, 4, 255] }),
    } as unknown as OffscreenCanvasRenderingContext2D;

    const result = sampleCanvasPixels(mockCtx, 320, 240);
    expect(result).toMatchObject({ darkSamples: 35, brightSamples: 0 });
  });

  it('returns an error object when getImageData throws', () => {
    const mockCtx = {
      getImageData: vi.fn().mockImplementation(() => {
        throw new Error('canvas tainted');
      }),
    } as unknown as OffscreenCanvasRenderingContext2D;

    const result = sampleCanvasPixels(mockCtx, 320, 240);
    expect(result).toMatchObject({ error: 'canvas tainted' });
  });
});
