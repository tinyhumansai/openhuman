/**
 * Tests for the `developerMode` field added to themeSlice and the
 * `selectDeveloperMode` selector.
 *
 * The field is persisted via the `theme` redux-persist config (localStorage,
 * same as other theme preferences).  These tests validate the reducer + selector
 * in isolation without requiring the full persist machinery.
 */
import { describe, expect, it } from 'vitest';

import themeReducer, { selectDeveloperMode, setDeveloperMode } from './themeSlice';

const initialState = themeReducer(undefined, { type: '@@INIT' });

describe('themeSlice — developerMode field', () => {
  it('defaults developerMode to false', () => {
    expect(initialState.developerMode).toBe(false);
  });

  it('enables developerMode via setDeveloperMode(true)', () => {
    const state = themeReducer(initialState, setDeveloperMode(true));
    expect(state.developerMode).toBe(true);
  });

  it('disables developerMode via setDeveloperMode(false)', () => {
    const withDevMode = themeReducer(initialState, setDeveloperMode(true));
    const back = themeReducer(withDevMode, setDeveloperMode(false));
    expect(back.developerMode).toBe(false);
  });

  it('leaves all other theme fields untouched when toggling developerMode', () => {
    const { developerMode: _, ...restBefore } = initialState;
    const withDevMode = themeReducer(initialState, setDeveloperMode(true));
    const { developerMode: _2, ...restAfter } = withDevMode;
    expect(restAfter).toEqual(restBefore);
  });
});

describe('selectDeveloperMode', () => {
  it('returns false from a fresh store', () => {
    const result = selectDeveloperMode({ theme: initialState });
    expect(result).toBe(false);
  });

  it('returns true after setDeveloperMode(true)', () => {
    const state = themeReducer(initialState, setDeveloperMode(true));
    expect(selectDeveloperMode({ theme: state })).toBe(true);
  });

  it('returns false after toggling back to false', () => {
    const enabled = themeReducer(initialState, setDeveloperMode(true));
    const disabled = themeReducer(enabled, setDeveloperMode(false));
    expect(selectDeveloperMode({ theme: disabled })).toBe(false);
  });
});
