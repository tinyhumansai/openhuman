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

  it('detects the legacy Indonesian browser locale code', async () => {
    vi.stubGlobal('navigator', { language: 'in-ID' });
    const reducer = await loadReducer();

    expect(reducer(undefined, { type: '@@INIT' }).current).toBe('id');
  });
});
