import { REHYDRATE } from 'redux-persist';
import { describe, expect, it } from 'vitest';

import reducer, {
  DEFAULT_MASCOT_COLOR,
  isCustomMascotGifUrl,
  MAX_CUSTOM_MASCOT_GIF_URL_LEN,
  MAX_MASCOT_VOICE_ID_LEN,
  selectCustomMascotGifUrl,
  selectMascotColor,
  selectMascotVoiceId,
  selectSelectedMascotId,
  setCustomMascotGifUrl,
  setMascotColor,
  setMascotVoiceId,
  setSelectedMascotId,
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
    const before = reducer(undefined, setMascotColor('navy'));
    // Cast: simulate a stale call (e.g. an older build dispatching a removed
    // variant) without weakening the public action signature.
    const after = reducer(before, setMascotColor('pink' as unknown as 'navy'));
    expect(after.color).toBe('navy');
  });

  it('resetUserScopedState resets back to default', () => {
    const dirty = reducer(undefined, setMascotColor('navy'));
    const reset = reducer(dirty, resetUserScopedState());
    expect(reset.color).toBe(DEFAULT_MASCOT_COLOR);
  });

  it('selectMascotColor reads the current color', () => {
    const state = reducer(undefined, setMascotColor('black'));
    expect(selectMascotColor({ mascot: state })).toBe('black');
  });

  it('exposes all five supported colors', () => {
    expect(new Set(SUPPORTED_MASCOT_COLORS)).toEqual(
      new Set(['yellow', 'burgundy', 'black', 'navy', 'custom'])
    );
  });

  describe('REHYDRATE', () => {
    const rehydrate = (key: string, payload?: unknown) => ({ type: REHYDRATE, key, payload });

    it('ignores REHYDRATE for a different persist key', () => {
      const initial = reducer(undefined, setMascotColor('navy'));
      const state = reducer(initial, rehydrate('other', { color: 'navy' }));
      expect(state.color).toBe('navy');
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
      const state = reducer(undefined, rehydrate('mascot', { color: 'navy' }));
      expect(state.color).toBe('navy');
      expect(state.voiceId).toBeNull();
    });
  });

  describe('selected backend mascot id', () => {
    it('defaults to null', () => {
      const state = reducer(undefined, { type: '@@INIT' });
      expect(state.selectedMascotId).toBeNull();
      expect(selectSelectedMascotId({ mascot: state })).toBeNull();
    });

    it('setSelectedMascotId trims and stores a valid id', () => {
      const state = reducer(undefined, setSelectedMascotId('  yellow  '));
      expect(state.selectedMascotId).toBe('yellow');
    });

    it('null payload clears the override', () => {
      let state = reducer(undefined, setSelectedMascotId('yellow'));
      state = reducer(state, setSelectedMascotId(null));
      expect(state.selectedMascotId).toBeNull();
    });

    it('empty / whitespace input is treated as a reset', () => {
      const state = reducer(
        reducer(undefined, setSelectedMascotId('yellow')),
        setSelectedMascotId('   ')
      );
      expect(state.selectedMascotId).toBeNull();
    });

    it('over-long input is dropped to null', () => {
      const tooLong = 'x'.repeat(MAX_MASCOT_VOICE_ID_LEN + 1);
      const state = reducer(undefined, setSelectedMascotId(tooLong));
      expect(state.selectedMascotId).toBeNull();
    });

    it('resetUserScopedState clears the override', () => {
      let state = reducer(undefined, setSelectedMascotId('yellow'));
      state = reducer(state, resetUserScopedState());
      expect(state.selectedMascotId).toBeNull();
    });

    const rehydrate = (key: string, payload?: unknown) => ({ type: REHYDRATE, key, payload });

    it('restores a valid persisted id', () => {
      const state = reducer(undefined, rehydrate('mascot', { selectedMascotId: 'yellow' }));
      expect(state.selectedMascotId).toBe('yellow');
    });

    it('scrubs an invalid persisted id back to null', () => {
      const state = reducer(undefined, rehydrate('mascot', { selectedMascotId: '   ' }));
      expect(state.selectedMascotId).toBeNull();
    });

    it('treats a missing selectedMascotId field (older builds) as null', () => {
      const state = reducer(undefined, rehydrate('mascot', { color: 'navy' }));
      expect(state.selectedMascotId).toBeNull();
    });
  });

  describe('custom mascot GIF avatar', () => {
    it('defaults to null', () => {
      const state = reducer(undefined, { type: '@@INIT' });
      expect(state.customMascotGifUrl).toBeNull();
      expect(selectCustomMascotGifUrl({ mascot: state })).toBeNull();
    });

    it('stores a trimmed HTTPS GIF URL', () => {
      const state = reducer(
        undefined,
        setCustomMascotGifUrl('  https://example.com/avatar.gif?size=2  ')
      );
      expect(state.customMascotGifUrl).toBe('https://example.com/avatar.gif?size=2');
    });

    it('accepts local GIF paths and loopback HTTP URLs', () => {
      expect(isCustomMascotGifUrl('/Users/me/avatar.gif')).toBe(true);
      expect(isCustomMascotGifUrl('~/Pictures/avatar.gif')).toBe(true);
      expect(isCustomMascotGifUrl('http://localhost/avatar.gif')).toBe(true);
      expect(isCustomMascotGifUrl('http://127.0.0.1/avatar.gif')).toBe(true);
    });

    it('rejects unsafe or non-GIF avatar sources', () => {
      expect(isCustomMascotGifUrl('javascript:alert(1)')).toBe(false);
      expect(isCustomMascotGifUrl('http://example.com/avatar.gif')).toBe(false);
      expect(isCustomMascotGifUrl('https://example.com/avatar.svg')).toBe(false);
      expect(isCustomMascotGifUrl('https://example.com/avatar.png')).toBe(false);
    });

    it('rejects oversize avatar sources', () => {
      const tooLong = `https://example.com/${'x'.repeat(MAX_CUSTOM_MASCOT_GIF_URL_LEN)}.gif`;
      const state = reducer(undefined, setCustomMascotGifUrl(tooLong));
      expect(state.customMascotGifUrl).toBeNull();
    });

    it('clears backend mascot id when a custom GIF is set', () => {
      let state = reducer(undefined, setSelectedMascotId('yellow'));
      state = reducer(state, setCustomMascotGifUrl('https://example.com/avatar.gif'));
      expect(state.customMascotGifUrl).toBe('https://example.com/avatar.gif');
      expect(state.selectedMascotId).toBeNull();
    });

    it('clears custom GIF when a backend mascot is selected', () => {
      let state = reducer(undefined, setCustomMascotGifUrl('https://example.com/avatar.gif'));
      state = reducer(state, setSelectedMascotId('yellow'));
      expect(state.selectedMascotId).toBe('yellow');
      expect(state.customMascotGifUrl).toBeNull();
    });

    it('resetUserScopedState clears a custom GIF avatar', () => {
      let state = reducer(undefined, setCustomMascotGifUrl('https://example.com/avatar.gif'));
      state = reducer(state, resetUserScopedState());
      expect(state.customMascotGifUrl).toBeNull();
    });

    const rehydrate = (key: string, payload?: unknown) => ({ type: REHYDRATE, key, payload });

    it('restores a valid persisted custom GIF avatar', () => {
      const state = reducer(
        undefined,
        rehydrate('mascot', { customMascotGifUrl: 'https://example.com/avatar.gif' })
      );
      expect(state.customMascotGifUrl).toBe('https://example.com/avatar.gif');
    });

    it('scrubs an invalid persisted custom GIF avatar back to null', () => {
      const state = reducer(
        undefined,
        rehydrate('mascot', { customMascotGifUrl: 'https://example.com/avatar.svg' })
      );
      expect(state.customMascotGifUrl).toBeNull();
    });
  });
});
