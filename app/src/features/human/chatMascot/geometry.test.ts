import { describe, expect, it } from 'vitest';

import {
  boxTransform,
  DOCK_PX,
  easeInOutCubic,
  inscribedSquare,
  lerpBox,
  snapBox,
  STAGE_RENDER_PX,
} from './geometry';

describe('inscribedSquare', () => {
  it('returns the rect unchanged when it is already square', () => {
    expect(inscribedSquare({ left: 10, top: 20, width: 64, height: 64 })).toEqual({
      left: 10,
      top: 20,
      size: 64,
    });
  });

  it('centres the largest square inside a wide rect', () => {
    // 200x80 → an 80px square, horizontally centred (60px of slack each side).
    expect(inscribedSquare({ left: 0, top: 0, width: 200, height: 80 })).toEqual({
      left: 60,
      top: 0,
      size: 80,
    });
  });

  it('centres the largest square inside a tall rect', () => {
    expect(inscribedSquare({ left: 0, top: 0, width: 80, height: 200 })).toEqual({
      left: 0,
      top: 60,
      size: 80,
    });
  });
});

describe('easeInOutCubic', () => {
  it('pins the endpoints', () => {
    expect(easeInOutCubic(0)).toBe(0);
    expect(easeInOutCubic(1)).toBe(1);
  });

  it('is symmetric about the midpoint', () => {
    expect(easeInOutCubic(0.5)).toBeCloseTo(0.5, 10);
    expect(easeInOutCubic(0.25) + easeInOutCubic(0.75)).toBeCloseTo(1, 10);
  });

  it('clamps out-of-range input rather than overshooting', () => {
    // The rAF loop can be handed a `t` past 1 when a frame lands late; an
    // unclamped cubic would fly the mascot past its anchor and snap back.
    expect(easeInOutCubic(1.4)).toBe(1);
    expect(easeInOutCubic(-0.3)).toBe(0);
  });
});

describe('lerpBox', () => {
  const dock = { left: 100, top: 400, size: 64 };
  const stage = { left: 900, top: 100, size: 420 };

  it('returns the endpoints exactly', () => {
    expect(lerpBox(dock, stage, 0)).toEqual(dock);
    expect(lerpBox(dock, stage, 1)).toEqual(stage);
  });

  it('interpolates position and size together', () => {
    expect(lerpBox(dock, stage, 0.5)).toEqual({ left: 500, top: 250, size: 242 });
  });
});

describe('snapBox', () => {
  it('rounds the position to whole pixels and leaves the size alone', () => {
    expect(snapBox({ left: 40.37, top: 499.62, size: 64.4 })).toEqual({
      left: 40,
      top: 500,
      size: 64.4,
    });
  });

  it('is a no-op on an already-whole position', () => {
    expect(snapBox({ left: 40, top: 500, size: 64 })).toEqual({ left: 40, top: 500, size: 64 });
  });
});

describe('STAGE_RENDER_PX', () => {
  it('is larger than any size the mascot is displayed at, so it only ever downscales', () => {
    // Upscaling a canvas by transform is a raster stretch — this is the invariant
    // that keeps the mascot sharp. The stage anchor is capped at 420px (see
    // ChatMascotStage) and the dock is DOCK_PX.
    const LARGEST_DISPLAYED_PX = 420;
    expect(STAGE_RENDER_PX).toBeGreaterThan(LARGEST_DISPLAYED_PX);
    expect(DOCK_PX).toBeLessThan(STAGE_RENDER_PX);
  });
});

describe('boxTransform', () => {
  it('maps the render box onto a target of the same size as an identity move', () => {
    expect(boxTransform({ left: 0, top: 0, size: STAGE_RENDER_PX })).toBe(
      'translate3d(0.00px, 0.00px, 0) scale(1.00000)'
    );
  });

  it('scales down onto the dock and translates to its top-left', () => {
    // 42/420 = 0.1 — the dock is a tenth of the render box.
    expect(boxTransform({ left: 12, top: 34, size: 42 }, 420)).toBe(
      'translate3d(12.00px, 34.00px, 0) scale(0.10000)'
    );
  });

  it('degrades to invisible rather than emitting a non-finite scale', () => {
    // A mis-measured frame must not produce `scale(Infinity)`, which would
    // blow the mascot up across the whole viewport for one frame.
    expect(boxTransform({ left: 0, top: 0, size: 64 }, 0)).toBe(
      'translate3d(0.00px, 0.00px, 0) scale(0.00000)'
    );
  });
});
