import { describe, expect, it } from 'vitest';

import { channelLuminance, channelsToHex, hexToChannels, isChannelTriple } from './color';

describe('theme colour helpers', () => {
  it('converts channel triples to hex', () => {
    expect(channelsToHex('47 110 244')).toBe('#2f6ef4');
    expect(channelsToHex('255 255 255')).toBe('#ffffff');
    expect(channelsToHex('0 0 0')).toBe('#000000');
  });

  it('converts hex to channel triples (3- and 6-digit)', () => {
    expect(hexToChannels('#2f6ef4')).toBe('47 110 244');
    expect(hexToChannels('2f6ef4')).toBe('47 110 244');
    expect(hexToChannels('#fff')).toBe('255 255 255');
  });

  it('round-trips hex → channels → hex', () => {
    for (const hex of ['#34c759', '#e8a728', '#ef4444', '#171717']) {
      expect(channelsToHex(hexToChannels(hex))).toBe(hex);
    }
  });

  it('degrades gracefully on malformed input', () => {
    expect(channelsToHex('not a colour')).toBe('#000000');
    expect(hexToChannels('zzz')).toBe('0 0 0');
  });

  it('validates channel triples', () => {
    expect(isChannelTriple('47 110 244')).toBe(true);
    expect(isChannelTriple('300 0 0')).toBe(false); // out of range
    expect(isChannelTriple('1 2')).toBe(false); // too few
    expect(isChannelTriple('#fff')).toBe(false);
  });

  it('computes luminance with white brighter than black', () => {
    expect(channelLuminance('255 255 255')).toBeGreaterThan(channelLuminance('0 0 0'));
    expect(channelLuminance('0 0 0')).toBeCloseTo(0, 5);
    expect(channelLuminance('255 255 255')).toBeCloseTo(1, 5);
  });
});
