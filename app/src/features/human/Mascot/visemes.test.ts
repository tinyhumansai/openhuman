import { describe, expect, it } from 'vitest';

import { lerpViseme, REST_SMILE_PATH, visemePath, VISEMES } from './visemes';

describe('visemes', () => {
  it('resting shape renders the smile path', () => {
    expect(visemePath(VISEMES.REST)).toBe(REST_SMILE_PATH);
  });

  it('open shapes render an oval-ish quadratic path', () => {
    const d = visemePath(VISEMES.A);
    expect(d.startsWith('M')).toBe(true);
    expect(d.endsWith('Z')).toBe(true);
    expect(d).toContain('Q');
    expect(d).not.toBe(REST_SMILE_PATH);
  });

  it('lerp at t=0 is shape a', () => {
    expect(lerpViseme(VISEMES.A, VISEMES.O, 0)).toEqual(VISEMES.A);
  });

  it('lerp at t=1 is shape b', () => {
    expect(lerpViseme(VISEMES.A, VISEMES.O, 1)).toEqual(VISEMES.O);
  });

  it('lerp at t=0.5 averages openness and width', () => {
    const out = lerpViseme(VISEMES.A, VISEMES.O, 0.5);
    expect(out.openness).toBeCloseTo((VISEMES.A.openness + VISEMES.O.openness) / 2);
    expect(out.width).toBeCloseTo((VISEMES.A.width + VISEMES.O.width) / 2);
  });

  it('lerp clamps t outside 0..1', () => {
    expect(lerpViseme(VISEMES.A, VISEMES.O, -1)).toEqual(VISEMES.A);
    expect(lerpViseme(VISEMES.A, VISEMES.O, 5)).toEqual(VISEMES.O);
  });

  it('width scales the path horizontal extent', () => {
    const narrow = visemePath({ openness: 0.5, width: 0 });
    const wide = visemePath({ openness: 0.5, width: 1 });
    expect(narrow).not.toBe(wide);
  });
});
