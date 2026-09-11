import { describe, expect, it } from 'vitest';

import { deriveSurfaceChrome, withDerivedChrome } from './chrome';
import { THEME_FAMILIES } from './presets';
import type { Theme } from './types';

const base = (over: Partial<Theme>): Theme => ({
  id: 't',
  name: 'T',
  isDark: false,
  builtIn: false,
  colors: {},
  fonts: {},
  ...over,
});

describe('deriveSurfaceChrome', () => {
  it('reproduces the tokens.css defaults from the default canvas', () => {
    // Light canvas 245 → chrome 214; dark canvas 0 → chrome 10. If these drift,
    // an untinted theme would derive a chrome that differs from the stylesheet.
    expect(deriveSurfaceChrome('245 245 245', false)).toBe('214 214 214');
    expect(deriveSurfaceChrome('0 0 0', true)).toBe('10 10 10');
  });

  it('keeps the light chrome darker than its canvas', () => {
    // Load-bearing: the chrome is painted at /30 as a legibility scrim over the
    // canvas, so a chrome equal to (or lighter than) it scrims nothing.
    const chrome = deriveSurfaceChrome('245 238 238', false)!;
    const [r] = chrome.split(' ').map(Number);
    expect(r).toBeLessThan(245);
  });

  it('keeps the dark chrome above its canvas', () => {
    const chrome = deriveSurfaceChrome('8 4 4', true)!;
    expect(chrome.split(' ').map(Number)).toEqual([18, 14, 14]);
  });

  it('preserves hue rather than collapsing to grey', () => {
    const [r, g, b] = deriveSurfaceChrome('245 238 238', false)!.split(' ').map(Number);
    expect(r).toBeGreaterThan(g);
    expect(g).toBe(b);
  });

  it('clamps instead of overflowing', () => {
    expect(deriveSurfaceChrome('250 250 250', true)).toBe('255 255 255');
  });

  it('returns undefined for a malformed triple', () => {
    expect(deriveSurfaceChrome('not a colour', false)).toBeUndefined();
    expect(deriveSurfaceChrome('1 2', false)).toBeUndefined();
  });
});

describe('withDerivedChrome', () => {
  it('never overrides an explicit value', () => {
    const theme = base({
      colors: {
        'surface-canvas': '245 238 238',
        'surface-chrome': '1 2 3',
        'line-strong': '9 9 9',
        'line-chrome': '4 5 6',
      },
    });
    const out = withDerivedChrome(theme);
    expect(out.colors['surface-chrome']).toBe('1 2 3');
    expect(out.colors['line-chrome']).toBe('4 5 6');
  });

  it('follows line-strong for line-chrome', () => {
    const out = withDerivedChrome(base({ colors: { 'line-strong': '226 200 200' } }));
    expect(out.colors['line-chrome']).toBe('226 200 200');
  });

  it('returns the same object when there is nothing to add', () => {
    // The built-in Light/Dark presets carry empty override maps; identity keeps
    // them usable as a render dependency without forcing a re-apply.
    const theme = base({});
    expect(withDerivedChrome(theme)).toBe(theme);
  });

  it('does not mutate its input', () => {
    const theme = base({ colors: { 'surface-canvas': '245 238 238' } });
    withDerivedChrome(theme);
    expect(theme.colors['surface-chrome']).toBeUndefined();
  });

  it('gives every themed preset a tinted chrome', () => {
    // The actual bug: presets tinted every surface except the frame around
    // them, so a red theme left the app background monotone grey.
    const presets = THEME_FAMILIES.flatMap(f => [f.light, f.dark]).filter(
      p => p && Object.keys(p.colors).length > 0
    ) as Theme[];
    expect(presets.length).toBeGreaterThan(0);

    for (const preset of presets) {
      const chrome = withDerivedChrome(preset).colors['surface-chrome'];
      expect(chrome, `${preset.id} should derive a chrome`).toBeDefined();
      const [r, g, b] = chrome!.split(' ').map(Number);
      const canvas = preset.colors['surface-canvas'].split(' ').map(Number);
      const canvasTinted = new Set(canvas).size > 1;
      // A tinted canvas must not derive a grey frame — that is the whole defect.
      if (canvasTinted) expect(new Set([r, g, b]).size).toBeGreaterThan(1);
    }
  });
});
