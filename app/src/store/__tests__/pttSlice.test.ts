import { describe, expect, it } from 'vitest';

import {
  initialPttState,
  pttReducer,
  setPttShortcut,
  setSpeakReplies,
  setShowOverlay,
  setIsHeld,
  type PttState,
} from '../pttSlice';
import { resetUserScopedState } from '../resetActions';

describe('ptt slice', () => {
  const initial: PttState = {
    shortcut: null,
    speakReplies: true,
    showOverlay: true,
    isHeld: false,
  };

  it('has the documented default state', () => {
    expect(pttReducer(undefined, { type: '@@INIT' })).toEqual(initial);
  });

  it('setPttShortcut stores the shortcut string', () => {
    const next = pttReducer(initial, setPttShortcut('F13'));
    expect(next.shortcut).toBe('F13');
  });

  it('setPttShortcut with null clears the shortcut', () => {
    const withKey: PttState = { ...initial, shortcut: 'F13' };
    const next = pttReducer(withKey, setPttShortcut(null));
    expect(next.shortcut).toBeNull();
  });

  it('setSpeakReplies toggles the flag', () => {
    expect(pttReducer(initial, setSpeakReplies(false)).speakReplies).toBe(false);
  });

  it('setShowOverlay toggles the flag', () => {
    expect(pttReducer(initial, setShowOverlay(false)).showOverlay).toBe(false);
  });

  it('setIsHeld updates the runtime hold flag', () => {
    expect(pttReducer(initial, setIsHeld(true)).isHeld).toBe(true);
  });

  it('resetUserScopedState returns the slice to initial state', () => {
    const dirty: PttState = {
      shortcut: 'F13',
      speakReplies: false,
      showOverlay: false,
      isHeld: true,
    };
    const next = pttReducer(dirty, resetUserScopedState());
    expect(next).toEqual(initialPttState);
  });
});
