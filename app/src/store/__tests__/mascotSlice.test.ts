import { REHYDRATE } from 'redux-persist';
import { describe, expect, it } from 'vitest';

import reducer, {
  DEFAULT_MASCOT_COLOR,
  selectMascotColor,
  setMascotColor,
  SUPPORTED_MASCOT_COLORS,
} from '../mascotSlice';
import { resetUserScopedState } from '../resetActions';

describe('mascotSlice', () => {
  it('starts with the default mascot color', () => {
    const state = reducer(undefined, { type: '@@INIT' });
    expect(state.color).toBe(DEFAULT_MASCOT_COLOR);
  });

  it('setMascotColor updates the color for supported variants', () => {
    let state = reducer(undefined, setMascotColor('navy'));
    expect(state.color).toBe('navy');
    state = reducer(state, setMascotColor('burgundy'));
    expect(state.color).toBe('burgundy');
  });

  it('setMascotColor ignores unknown variants', () => {
    const before = reducer(undefined, setMascotColor('green'));
    // Cast: simulate a stale call (e.g. an older build dispatching a removed
    // variant) without weakening the public action signature.
    const after = reducer(before, setMascotColor('pink' as unknown as 'green'));
    expect(after.color).toBe('green');
  });

  it('resetUserScopedState resets back to default', () => {
    const dirty = reducer(undefined, setMascotColor('green'));
    const reset = reducer(dirty, resetUserScopedState());
    expect(reset.color).toBe(DEFAULT_MASCOT_COLOR);
  });

  it('selectMascotColor reads the current color', () => {
    const state = reducer(undefined, setMascotColor('black'));
    expect(selectMascotColor({ mascot: state })).toBe('black');
  });

  it('exposes all five supported colors', () => {
    expect(new Set(SUPPORTED_MASCOT_COLORS)).toEqual(
      new Set(['yellow', 'burgundy', 'black', 'navy', 'green'])
    );
  });

  describe('REHYDRATE', () => {
    const rehydrate = (key: string, payload?: unknown) => ({ type: REHYDRATE, key, payload });

    it('ignores REHYDRATE for a different persist key', () => {
      const initial = reducer(undefined, setMascotColor('green'));
      const state = reducer(initial, rehydrate('other', { color: 'navy' }));
      expect(state.color).toBe('green');
    });

    it('restores a valid persisted color for the mascot key', () => {
      const state = reducer(undefined, rehydrate('mascot', { color: 'burgundy' }));
      expect(state.color).toBe('burgundy');
    });

    it('falls back to the default when the persisted color is unknown', () => {
      const state = reducer(undefined, rehydrate('mascot', { color: 'fuchsia' }));
      expect(state.color).toBe(DEFAULT_MASCOT_COLOR);
    });

    it('falls back to the default when no payload is present', () => {
      const state = reducer(undefined, rehydrate('mascot'));
      expect(state.color).toBe(DEFAULT_MASCOT_COLOR);
    });
  });
});
