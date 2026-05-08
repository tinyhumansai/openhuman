import { describe, expect, it, vi } from 'vitest';

import reducer, { resetCoreMode, setCoreMode } from './coreModeSlice';

describe('coreModeSlice', () => {
  it('initialises to unset', () => {
    const state = reducer(undefined, { type: '@@INIT' });
    expect(state.mode).toEqual({ kind: 'unset' });
  });

  it('sets local mode', () => {
    const state = reducer(undefined, setCoreMode({ kind: 'local' }));
    expect(state.mode).toEqual({ kind: 'local' });
  });

  it('sets cloud mode with url', () => {
    const state = reducer(
      undefined,
      setCoreMode({ kind: 'cloud', url: 'https://core.example.com/rpc' })
    );
    expect(state.mode).toEqual({ kind: 'cloud', url: 'https://core.example.com/rpc' });
  });

  it('sets cloud mode with url + token', () => {
    const state = reducer(
      undefined,
      setCoreMode({ kind: 'cloud', url: 'https://core.example.com/rpc', token: 'tok-1234' })
    );
    expect(state.mode).toEqual({
      kind: 'cloud',
      url: 'https://core.example.com/rpc',
      token: 'tok-1234',
    });
  });

  it('resets to unset', () => {
    const withLocal = reducer(undefined, setCoreMode({ kind: 'local' }));
    const reset = reducer(withLocal, resetCoreMode());
    expect(reset.mode).toEqual({ kind: 'unset' });
  });

  it('overwrites previous mode on setCoreMode', () => {
    const withCloud = reducer(
      undefined,
      setCoreMode({ kind: 'cloud', url: 'https://old.example.com' })
    );
    const withLocal = reducer(withCloud, setCoreMode({ kind: 'local' }));
    expect(withLocal.mode).toEqual({ kind: 'local' });
  });

  it('slice name is coreMode', () => {
    // Structural assertion: the key used by redux-persist must match the
    // persist config key declared in store/index.ts.
    expect(setCoreMode.type).toMatch(/^coreMode\//);
  });
});

describe('coreModeSlice — sync-localStorage-derived initial state', () => {
  // The slice's initialState comes from `deriveInitialMode()` which reads
  // `localStorage` at module load. We re-import per test to exercise each
  // branch of that derivation.
  async function freshImport() {
    vi.resetModules();
    return import('./coreModeSlice');
  }

  it('hydrates to local when openhuman_core_mode=local', async () => {
    localStorage.clear();
    localStorage.setItem('openhuman_core_mode', 'local');
    const mod = await freshImport();
    const state = mod.default(undefined, { type: '@@INIT' });
    expect(state.mode).toEqual({ kind: 'local' });
  });

  it('hydrates to cloud with url + token when all three keys are present', async () => {
    localStorage.clear();
    localStorage.setItem('openhuman_core_mode', 'cloud');
    localStorage.setItem('openhuman_core_rpc_url', 'https://core.example.com/rpc');
    localStorage.setItem('openhuman_core_rpc_token', 'tok-abc');
    const mod = await freshImport();
    const state = mod.default(undefined, { type: '@@INIT' });
    expect(state.mode).toEqual({
      kind: 'cloud',
      url: 'https://core.example.com/rpc',
      token: 'tok-abc',
    });
  });

  it('falls back to unset when cloud marker exists but URL or token is missing', async () => {
    localStorage.clear();
    localStorage.setItem('openhuman_core_mode', 'cloud');
    localStorage.setItem('openhuman_core_rpc_url', 'https://core.example.com/rpc');
    // Token deliberately missing.
    const mod = await freshImport();
    const state = mod.default(undefined, { type: '@@INIT' });
    expect(state.mode).toEqual({ kind: 'unset' });
  });

  it('returns unset when no marker is stored', async () => {
    localStorage.clear();
    const mod = await freshImport();
    const state = mod.default(undefined, { type: '@@INIT' });
    expect(state.mode).toEqual({ kind: 'unset' });
  });
});
