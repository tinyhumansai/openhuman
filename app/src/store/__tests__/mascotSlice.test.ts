import { REHYDRATE } from 'redux-persist';
import { describe, expect, it } from 'vitest';

import reducer, {
  DEFAULT_MASCOT_COLOR,
  MAX_MASCOT_VOICE_ID_LEN,
  selectMascotColor,
  selectMascotVoiceId,
  setMascotColor,
  setMascotVoiceId,
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

  // Issue #1762 — user-selected ElevenLabs voice id for the mascot's
  // reply speech. The slice is the single source of truth; the
  // VoicePanel writes through here and `useHumanMascot` reads back.
  describe('mascot voice id', () => {
    it('starts with no override (null)', () => {
      const state = reducer(undefined, { type: '@@INIT' });
      expect(state.voiceId).toBeNull();
      expect(selectMascotVoiceId({ mascot: state })).toBeNull();
    });

    it('setMascotVoiceId stores a trimmed non-empty id', () => {
      const state = reducer(undefined, setMascotVoiceId('  21m00Tcm4TlvDq8ikWAM  '));
      expect(state.voiceId).toBe('21m00Tcm4TlvDq8ikWAM');
    });

    it('setMascotVoiceId(null) clears the override', () => {
      const set = reducer(undefined, setMascotVoiceId('21m00Tcm4TlvDq8ikWAM'));
      const cleared = reducer(set, setMascotVoiceId(null));
      expect(cleared.voiceId).toBeNull();
    });

    it('setMascotVoiceId resets on whitespace-only input rather than storing junk', () => {
      const initial = reducer(undefined, setMascotVoiceId('valid-id'));
      const blanked = reducer(initial, setMascotVoiceId('   '));
      expect(blanked.voiceId).toBeNull();
    });

    it('setMascotVoiceId rejects oversize payloads', () => {
      const huge = 'x'.repeat(MAX_MASCOT_VOICE_ID_LEN + 1);
      const state = reducer(undefined, setMascotVoiceId(huge));
      expect(state.voiceId).toBeNull();
    });

    it('resetUserScopedState clears any voice id override', () => {
      const dirty = reducer(undefined, setMascotVoiceId('custom-voice'));
      expect(dirty.voiceId).toBe('custom-voice');
      const reset = reducer(dirty, resetUserScopedState());
      expect(reset.voiceId).toBeNull();
    });
  });

  describe('REHYDRATE — mascot voice id', () => {
    const rehydrate = (key: string, payload?: unknown) => ({ type: REHYDRATE, key, payload });

    it('restores a valid persisted voice id', () => {
      const state = reducer(
        undefined,
        rehydrate('mascot', { color: 'navy', voiceId: 'persisted-id' })
      );
      expect(state.voiceId).toBe('persisted-id');
    });

    it('scrubs an invalid persisted voice id back to null', () => {
      const state = reducer(undefined, rehydrate('mascot', { color: 'navy', voiceId: '   ' }));
      expect(state.voiceId).toBeNull();
    });

    it('treats a missing voiceId field (older builds) as null', () => {
      // Pre-#1762 blobs only carry `color`; the slice must not throw or
      // crash on missing keys — that would brick rehydrate for everyone
      // on an upgrade.
      const state = reducer(undefined, rehydrate('mascot', { color: 'green' }));
      expect(state.color).toBe('green');
      expect(state.voiceId).toBeNull();
    });
  });
});
