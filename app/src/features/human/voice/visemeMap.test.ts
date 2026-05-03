import { describe, expect, it } from 'vitest';

import { VISEMES } from '../Mascot/visemes';
import { findActiveFrame, oculusVisemeToShape } from './visemeMap';

describe('oculusVisemeToShape', () => {
  it('maps silence to rest', () => {
    expect(oculusVisemeToShape('sil')).toBe(VISEMES.REST);
  });

  it('maps bilabial PP to closed M', () => {
    expect(oculusVisemeToShape('PP')).toBe(VISEMES.M);
  });

  it('maps FF to F', () => {
    expect(oculusVisemeToShape('FF')).toBe(VISEMES.F);
  });

  it('maps the five vowels to their shapes', () => {
    expect(oculusVisemeToShape('aa')).toBe(VISEMES.A);
    expect(oculusVisemeToShape('E')).toBe(VISEMES.E);
    expect(oculusVisemeToShape('I')).toBe(VISEMES.I);
    expect(oculusVisemeToShape('O')).toBe(VISEMES.O);
    expect(oculusVisemeToShape('U')).toBe(VISEMES.U);
  });

  it('returns valid shape for every consonant code', () => {
    for (const code of ['TH', 'DD', 'kk', 'CH', 'SS', 'nn', 'RR']) {
      const s = oculusVisemeToShape(code);
      expect(s.openness).toBeGreaterThanOrEqual(0);
      expect(s.openness).toBeLessThanOrEqual(1);
      expect(s.width).toBeGreaterThanOrEqual(0);
      expect(s.width).toBeLessThanOrEqual(1);
    }
  });

  it('falls back to REST for unknown codes', () => {
    expect(oculusVisemeToShape('zzz')).toBe(VISEMES.REST);
    expect(oculusVisemeToShape('')).toBe(VISEMES.REST);
  });
});

describe('findActiveFrame', () => {
  const frames = [
    { viseme: 'sil', start_ms: 0, end_ms: 100 },
    { viseme: 'aa', start_ms: 100, end_ms: 250 },
    { viseme: 'PP', start_ms: 250, end_ms: 400 },
    { viseme: 'O', start_ms: 400, end_ms: 600 },
  ];

  it('returns null on empty input', () => {
    expect(findActiveFrame([], 50).frame).toBeNull();
  });

  it('finds the frame at a given time', () => {
    expect(findActiveFrame(frames, 50).frame?.viseme).toBe('sil');
    expect(findActiveFrame(frames, 200).frame?.viseme).toBe('aa');
    expect(findActiveFrame(frames, 500).frame?.viseme).toBe('O');
  });

  it('returns null past the end of the timeline', () => {
    expect(findActiveFrame(frames, 1000).frame).toBeNull();
  });

  it('advances the cursor on monotonic playback', () => {
    let cursor = 0;
    ({ cursor } = findActiveFrame(frames, 50, cursor));
    expect(cursor).toBe(0);
    ({ cursor } = findActiveFrame(frames, 200, cursor));
    expect(cursor).toBe(1);
    ({ cursor } = findActiveFrame(frames, 500, cursor));
    expect(cursor).toBe(3);
  });

  it('clamps a negative cursor to 0 to avoid undefined-frame access', () => {
    const result = findActiveFrame(frames, 50, -10);
    expect(result.frame?.viseme).toBe('sil');
    expect(result.cursor).toBeGreaterThanOrEqual(0);
  });

  it('rewinds the cursor when time jumps backward', () => {
    const { cursor } = findActiveFrame(frames, 500, 0);
    const back = findActiveFrame(frames, 50, cursor);
    expect(back.frame?.viseme).toBe('sil');
  });
});
