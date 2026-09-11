import { describe, expect, it } from 'vitest';

import { amplitudeToVisemeCode, smoothAmplitude } from './amplitudeLipsync';

describe('amplitudeToVisemeCode', () => {
  it('rests the mouth on silence and room tone', () => {
    expect(amplitudeToVisemeCode(0)).toBe('sil');
    // Below the floor: the tail of a word, not speech. Holding the mouth open
    // through these gaps is what makes naive amplitude lip-sync look slack.
    expect(amplitudeToVisemeCode(0.03)).toBe('sil');
  });

  it('opens the mouth further as the signal gets louder', () => {
    const codes = [0.08, 0.2, 0.6].map(amplitudeToVisemeCode);
    expect(codes).toEqual(['I', 'E', 'aa']);
  });

  // A garbage reading (analyser torn down mid-frame) must not map to a wide-open
  // mouth that then sticks for the rest of the call. Infinity rests for the same
  // reason NaN does: it is a broken sample, not a loud one.
  it('rests on a non-finite reading', () => {
    expect(amplitudeToVisemeCode(Number.NaN)).toBe('sil');
    expect(amplitudeToVisemeCode(Number.POSITIVE_INFINITY)).toBe('sil');
  });
});

describe('smoothAmplitude', () => {
  it('moves toward the new sample', () => {
    expect(smoothAmplitude(0, 1)).toBeGreaterThan(0);
    expect(smoothAmplitude(1, 0)).toBeLessThan(1);
  });

  // Asymmetric on purpose: consonant onsets must land on time, but the mouth
  // must not snap shut inside a word.
  it('opens faster than it closes', () => {
    const opening = smoothAmplitude(0, 1) - 0;
    const closing = 1 - smoothAmplitude(1, 0);
    expect(opening).toBeGreaterThan(closing);
  });

  it('converges on a held level rather than oscillating', () => {
    let level = 0;
    for (let i = 0; i < 40; i += 1) level = smoothAmplitude(level, 0.5);
    expect(level).toBeCloseTo(0.5, 2);
  });
});
