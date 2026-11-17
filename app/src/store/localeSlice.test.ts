import { afterEach, describe, expect, it, vi } from 'vitest';

async function loadReducer() {
  vi.resetModules();
  const mod = await import('./localeSlice');
  return mod.default;
}

describe('localeSlice', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('detects Indonesian browser locales', async () => {
    vi.stubGlobal('navigator', { language: 'id-ID' });
    const reducer = await loadReducer();

    expect(reducer(undefined, { type: '@@INIT' }).current).toBe('id');
  });

  it('detects German browser locales', async () => {
    vi.stubGlobal('navigator', { language: 'de-DE' });
    const reducer = await loadReducer();

    expect(reducer(undefined, { type: '@@INIT' }).current).toBe('de');
  });

  it('detects the legacy Indonesian browser locale code', async () => {
    vi.stubGlobal('navigator', { language: 'in-ID' });
    const reducer = await loadReducer();

    expect(reducer(undefined, { type: '@@INIT' }).current).toBe('id');
  });

  it.each(['tr', 'tr-TR', 'TR-tr'])('detects the Turkish browser locale %s', async language => {
    vi.stubGlobal('navigator', { language });
    const reducer = await loadReducer();

    expect(reducer(undefined, { type: '@@INIT' }).current).toBe('tr');
  });
});

describe('Japanese locale detection', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it.each(['ja', 'ja-JP', 'JA-jp'])('detects browser language %s', async language => {
    vi.stubGlobal('navigator', { language });
    const reducer = await loadReducer();

    expect(reducer(undefined, { type: '@@INIT' }).current).toBe('ja');
  });

  it.each(['', 'unsupported'])('keeps the English fallback for %s', async language => {
    vi.stubGlobal('navigator', { language });
    const reducer = await loadReducer();

    expect(reducer(undefined, { type: '@@INIT' }).current).toBe('en');
  });

  it('keeps the English fallback without the browser API', async () => {
    vi.stubGlobal('navigator', undefined);
    const reducer = await loadReducer();

    expect(reducer(undefined, { type: '@@INIT' }).current).toBe('en');
  });
});
