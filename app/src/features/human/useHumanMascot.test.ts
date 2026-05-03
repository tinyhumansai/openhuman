import { describe, expect, it } from 'vitest';

import { VISEMES } from './Mascot/visemes';
import { pickViseme } from './useHumanMascot';

describe('pickViseme', () => {
  it('maps vowels to their viseme', () => {
    expect(pickViseme('a')).toBe(VISEMES.A);
    expect(pickViseme('e')).toBe(VISEMES.E);
    expect(pickViseme('i')).toBe(VISEMES.I);
    expect(pickViseme('o')).toBe(VISEMES.O);
    expect(pickViseme('u')).toBe(VISEMES.U);
  });

  it('maps labials to M', () => {
    expect(pickViseme('m')).toBe(VISEMES.M);
    expect(pickViseme('b')).toBe(VISEMES.M);
    expect(pickViseme('p')).toBe(VISEMES.M);
  });

  it('maps fricatives to F', () => {
    expect(pickViseme('f')).toBe(VISEMES.F);
    expect(pickViseme('v')).toBe(VISEMES.F);
  });

  it('uses the trailing letter of multi-char deltas', () => {
    expect(pickViseme('hello')).toBe(VISEMES.O);
    expect(pickViseme('world')).toBe(VISEMES.E); // d → fallback
  });

  it('ignores punctuation when picking the trailing letter', () => {
    expect(pickViseme('Hi!')).toBe(VISEMES.I);
    expect(pickViseme('...')).toBe(VISEMES.E); // no letters → fallback
  });

  it('falls back to E for unmapped consonants', () => {
    expect(pickViseme('z')).toBe(VISEMES.E);
    expect(pickViseme('')).toBe(VISEMES.E);
  });
});
