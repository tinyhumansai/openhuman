import { describe, expect, it } from 'vitest';

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
