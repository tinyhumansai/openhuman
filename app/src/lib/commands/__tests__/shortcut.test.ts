import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { formatShortcut, matchEvent, parseShortcut } from '../shortcut';

describe('parseShortcut', () => {
  it('parses mod+k', () => {
    expect(parseShortcut('mod+k')).toEqual({
      key: 'k',
      mod: true,
      shift: false,
      alt: false,
      ctrl: false,
    });
  });
  it('parses shift+mod+p', () => {
    expect(parseShortcut('shift+mod+p')).toEqual({
      key: 'p',
      mod: true,
      shift: true,
      alt: false,
      ctrl: false,
    });
  });
  it('parses ?', () => {
    expect(parseShortcut('?')).toEqual({
      key: '?',
      mod: false,
      shift: false,
      alt: false,
      ctrl: false,
    });
  });
  it('parses escape and f1 and arrowup', () => {
    expect(parseShortcut('escape').key).toBe('escape');
    expect(parseShortcut('f1').key).toBe('f1');
    expect(parseShortcut('arrowup').key).toBe('arrowup');
  });
  it('parses mod+,', () => {
    expect(parseShortcut('mod+,')).toEqual({
      key: ',',
      mod: true,
      shift: false,
      alt: false,
      ctrl: false,
    });
  });
  it('throws on empty', () => {
    expect(() => parseShortcut('')).toThrow();
  });
  it('throws on modifier-only', () => {
    expect(() => parseShortcut('mod')).toThrow();
  });
  it('throws on meta+k (must use mod)', () => {
    expect(() => parseShortcut('meta+k')).toThrow();
  });
  it('memoizes', () => {
    expect(parseShortcut('mod+k')).toBe(parseShortcut('mod+k'));
  });
});

function ke(opts: Partial<KeyboardEventInit> & { key: string }): KeyboardEvent {
  return new KeyboardEvent('keydown', {
    key: opts.key,
    metaKey: !!opts.metaKey,
    ctrlKey: !!opts.ctrlKey,
    shiftKey: !!opts.shiftKey,
    altKey: !!opts.altKey,
  });
}

describe('matchEvent (mac)', () => {
  const origPlatform = navigator.platform;
  beforeAll(() => {
    Object.defineProperty(navigator, 'platform', { value: 'MacIntel', configurable: true });
  });
  afterAll(() => {
    Object.defineProperty(navigator, 'platform', { value: origPlatform, configurable: true });
  });

  it('mod+k matches metaKey+k', () => {
    expect(matchEvent(parseShortcut('mod+k'), ke({ key: 'k', metaKey: true }))).toBe(true);
  });
  it('mod+k does NOT match ctrlKey+k on mac', () => {
    expect(matchEvent(parseShortcut('mod+k'), ke({ key: 'k', ctrlKey: true }))).toBe(false);
  });
  it('k does not match shift+k', () => {
    expect(matchEvent(parseShortcut('k'), ke({ key: 'K', shiftKey: true }))).toBe(false);
  });
  it('? matches e.key === "?"', () => {
    expect(matchEvent(parseShortcut('?'), ke({ key: '?' }))).toBe(true);
  });
  it('escape matches Escape', () => {
    expect(matchEvent(parseShortcut('escape'), ke({ key: 'Escape' }))).toBe(true);
  });
});

describe('matchEvent (non-mac)', () => {
  const origPlatform = navigator.platform;
  beforeAll(() => {
    Object.defineProperty(navigator, 'platform', { value: 'Win32', configurable: true });
  });
  afterAll(() => {
    Object.defineProperty(navigator, 'platform', { value: origPlatform, configurable: true });
  });

  it('mod+k matches ctrlKey+k', () => {
    expect(matchEvent(parseShortcut('mod+k'), ke({ key: 'k', ctrlKey: true }))).toBe(true);
  });
});

describe('formatShortcut', () => {
  it('mac renders glyphs', () => {
    expect(formatShortcut(parseShortcut('shift+mod+k'), true)).toEqual(['⇧', '⌘', 'K']);
  });
  it('pc renders labels', () => {
    expect(formatShortcut(parseShortcut('shift+mod+k'), false)).toEqual(['Shift', 'Ctrl', 'K']);
  });
  it('single printable renders alone', () => {
    expect(formatShortcut(parseShortcut('?'), true)).toEqual(['?']);
  });
});
