import { describe, expect, it } from 'vitest';

import {
  availablePoses,
  initialChannelValues,
  pickIdleFlourish,
  resolveFaceToPose,
  resolveVisemeCode,
  restVisemeCode,
} from './stateEngine';
import type { MascotStateEngine } from './types';

const TINY: MascotStateEngine = {
  idlePoseCycle: ['idle', 'bookreading', 'coffeedrink', 'writing', 'dancing'],
  states: { idle: 'idle', thinking: 'thinking' },
  visemeCodes: ['sil', 'PP', 'FF', 'aa', 'E', 'ih', 'oh', 'ou'],
};

// A sparse mascot: distinct logical state names, no shared flourish vocabulary.
const TOSHI: MascotStateEngine = {
  idlePoseCycle: ['idle', 'look_around', 'pointing'],
  states: { idle: 'idle', thinking: 'look_around' },
  visemeCodes: ['sil', 'PP', 'aa'],
  channels: [
    {
      key: 'eyes',
      label: 'Eyes',
      values: ['blink', 'look_left', 'look_right'],
      default: 'look_left',
      cycle: { enabled: true, intervalMs: 2600, order: 'random' },
    },
  ],
};

describe('availablePoses', () => {
  it('unions idlePoseCycle and logical state values', () => {
    expect(availablePoses(TOSHI)).toEqual(new Set(['idle', 'look_around', 'pointing']));
  });
});

describe('resolveFaceToPose', () => {
  it('uses the desired pose when the mascot has it', () => {
    expect(resolveFaceToPose('writing', TINY)).toBe('writing');
    expect(resolveFaceToPose('reading', TINY)).toBe('bookreading');
  });

  it('uses manifest logical state mappings for custom activity poses', () => {
    const custom: MascotStateEngine = {
      ...TOSHI,
      states: { ...TOSHI.states, writing: 'scribbling' },
      idlePoseCycle: [...TOSHI.idlePoseCycle, 'scribbling'],
    };

    expect(resolveFaceToPose('writing', custom)).toBe('scribbling');
  });

  it('falls back to the thinking state for thinking-ish faces', () => {
    // Toshi has no 'thinking' pose; thinking-ish faces map to its 'look_around'.
    expect(resolveFaceToPose('thinking', TOSHI)).toBe('look_around');
    expect(resolveFaceToPose('confused', TOSHI)).toBe('look_around');
  });

  it('falls back to the idle state when the flourish is absent', () => {
    // Toshi has no 'writing' pose → rest on idle.
    expect(resolveFaceToPose('writing', TOSHI)).toBe('idle');
  });
});

describe('resolveVisemeCode', () => {
  it('normalises and keeps codes in the mascot vocabulary', () => {
    expect(resolveVisemeCode('O', TINY)).toBe('oh');
    expect(resolveVisemeCode('E', TINY)).toBe('E');
  });

  it("returns the manifest's exact casing for case-varied viseme enums", () => {
    const upper: MascotStateEngine = { ...TINY, visemeCodes: ['SIL', 'PP', 'AA', 'OH'] };
    expect(resolveVisemeCode('O', upper)).toBe('OH');
    expect(resolveVisemeCode('a', upper)).toBe('AA');
    expect(restVisemeCode(upper)).toBe('SIL');
  });

  it('preserves raw close-vowel aliases when the manifest uses them', () => {
    const raw: MascotStateEngine = { ...TINY, visemeCodes: ['sil', 'I', 'O', 'U'] };
    expect(resolveVisemeCode('I', raw)).toBe('I');
    expect(resolveVisemeCode('O', raw)).toBe('O');
    expect(resolveVisemeCode('U', raw)).toBe('U');
  });

  it('falls back to rest for codes the mascot lacks', () => {
    // Toshi has no 'oh'; an O viseme rests the mouth instead of no-op.
    expect(resolveVisemeCode('O', TOSHI)).toBe('sil');
    expect(resolveVisemeCode('???', TINY)).toBe('sil');
  });

  it('restVisemeCode prefers sil', () => {
    expect(restVisemeCode(TINY)).toBe('sil');
    expect(restVisemeCode({ ...TINY, visemeCodes: ['rest', 'aa'] })).toBe('rest');
  });
});

describe('pickIdleFlourish', () => {
  it('never returns the resting idle pose', () => {
    for (let r = 0; r < 1; r += 0.1) {
      expect(pickIdleFlourish(TINY, undefined, () => r)).not.toBe('idle');
    }
  });

  it('avoids the excluded (just-played) pose', () => {
    // rng=0 would pick the first non-idle pose ('bookreading'); excluding it
    // shifts the pool so the first pick is the next one.
    expect(pickIdleFlourish(TINY, 'bookreading', () => 0)).toBe('coffeedrink');
  });

  it('returns the rest pose when there are no flourishes', () => {
    const flat: MascotStateEngine = { ...TINY, idlePoseCycle: ['idle'] };
    expect(pickIdleFlourish(flat)).toBe('idle');
  });
});

describe('initialChannelValues', () => {
  it('uses default then first value', () => {
    expect(initialChannelValues(TOSHI)).toEqual({ eyes: 'look_left' });
    expect(
      initialChannelValues({ ...TOSHI, channels: [{ key: 'eyes', values: ['blink', 'open'] }] })
    ).toEqual({ eyes: 'blink' });
  });

  it('is empty when there are no channels', () => {
    expect(initialChannelValues(TINY)).toEqual({});
  });
});
