import { REHYDRATE } from 'redux-persist';
import { describe, expect, it } from 'vitest';

import reducer, {
  DEFAULT_MASCOT_COLOR,
  isCustomMascotGifUrl,
  MAX_CUSTOM_MASCOT_GIF_URL_LEN,
  MAX_MASCOT_VOICE_ID_LEN,
  MAX_MASCOT_VOICES,
  selectCustomMascotGifUrl,
  selectDualMascotEnabled,
  selectMascotColor,
  selectMascotVoiceFor,
  selectMascotVoiceId,
  selectMascotVoices,
  selectMeetingMascotVoicePair,
  selectSecondaryMascotId,
  selectSelectedMascotId,
  setCustomMascotGifUrl,
  setMascotColor,
  setMascotVoice,
  setMascotVoiceId,
  setSecondaryMascotId,
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

  // Issue #4277 — second meeting mascot + per-mascot voices.
  describe('secondary mascot id', () => {
    it('defaults to null', () => {
      const state = reducer(undefined, { type: '@@INIT' });
      expect(state.secondaryMascotId).toBeNull();
      expect(selectSecondaryMascotId({ mascot: state })).toBeNull();
    });

    it('setSecondaryMascotId trims and stores a valid id', () => {
      const state = reducer(undefined, setSecondaryMascotId('  toshi  '));
      expect(state.secondaryMascotId).toBe('toshi');
    });

    it('null / whitespace / oversize input clears it', () => {
      const set = reducer(undefined, setSecondaryMascotId('toshi'));
      expect(reducer(set, setSecondaryMascotId(null)).secondaryMascotId).toBeNull();
      expect(reducer(set, setSecondaryMascotId('   ')).secondaryMascotId).toBeNull();
      const tooLong = 'x'.repeat(MAX_MASCOT_VOICE_ID_LEN + 1);
      expect(reducer(set, setSecondaryMascotId(tooLong)).secondaryMascotId).toBeNull();
    });

    it('is cleared when a custom GIF avatar is set (mutually exclusive)', () => {
      let state = reducer(undefined, setSecondaryMascotId('toshi'));
      state = reducer(state, setCustomMascotGifUrl('https://example.com/avatar.gif'));
      expect(state.secondaryMascotId).toBeNull();
    });

    it('resetUserScopedState clears it', () => {
      let state = reducer(undefined, setSecondaryMascotId('toshi'));
      state = reducer(state, resetUserScopedState());
      expect(state.secondaryMascotId).toBeNull();
    });

    const rehydrate = (key: string, payload?: unknown) => ({ type: REHYDRATE, key, payload });

    it('restores a valid persisted id; scrubs invalid; tolerates missing (older blobs)', () => {
      expect(
        reducer(undefined, rehydrate('mascot', { secondaryMascotId: 'toshi' })).secondaryMascotId
      ).toBe('toshi');
      expect(
        reducer(undefined, rehydrate('mascot', { secondaryMascotId: '  ' })).secondaryMascotId
      ).toBeNull();
      expect(
        reducer(undefined, rehydrate('mascot', { color: 'navy' })).secondaryMascotId
      ).toBeNull();
    });
  });

  describe('per-mascot voices', () => {
    it('defaults to an empty map', () => {
      const state = reducer(undefined, { type: '@@INIT' });
      expect(state.mascotVoices).toEqual({});
      expect(selectMascotVoices({ mascot: state })).toEqual({});
      expect(selectMascotVoiceFor('toshi')({ mascot: state })).toBeNull();
    });

    it('setMascotVoice records a trimmed mascotId → voiceId entry', () => {
      const state = reducer(undefined, setMascotVoice({ mascotId: ' toshi ', voiceId: '  v-1  ' }));
      expect(state.mascotVoices).toEqual({ toshi: 'v-1' });
      expect(selectMascotVoiceFor('toshi')({ mascot: state })).toBe('v-1');
    });

    it('setMascotVoice with null / invalid voiceId removes the entry', () => {
      const set = reducer(undefined, setMascotVoice({ mascotId: 'toshi', voiceId: 'v-1' }));
      expect(
        reducer(set, setMascotVoice({ mascotId: 'toshi', voiceId: null })).mascotVoices
      ).toEqual({});
      expect(
        reducer(set, setMascotVoice({ mascotId: 'toshi', voiceId: '   ' })).mascotVoices
      ).toEqual({});
    });

    it('ignores an invalid mascotId key', () => {
      const state = reducer(undefined, setMascotVoice({ mascotId: '   ', voiceId: 'v-1' }));
      expect(state.mascotVoices).toEqual({});
    });

    it('caps new keys at MAX_MASCOT_VOICES but still updates existing ones', () => {
      let state = reducer(undefined, { type: '@@INIT' });
      for (let i = 0; i < MAX_MASCOT_VOICES; i += 1) {
        state = reducer(state, setMascotVoice({ mascotId: `m-${i}`, voiceId: `v-${i}` }));
      }
      expect(Object.keys(state.mascotVoices)).toHaveLength(MAX_MASCOT_VOICES);
      // A brand-new key over the cap is refused…
      const overflow = reducer(state, setMascotVoice({ mascotId: 'm-extra', voiceId: 'v-x' }));
      expect(overflow.mascotVoices['m-extra']).toBeUndefined();
      // …but re-voicing an existing mascot is always allowed.
      const updated = reducer(state, setMascotVoice({ mascotId: 'm-0', voiceId: 'v-new' }));
      expect(updated.mascotVoices['m-0']).toBe('v-new');
    });

    it('resetUserScopedState clears the map', () => {
      const dirty = reducer(undefined, setMascotVoice({ mascotId: 'toshi', voiceId: 'v-1' }));
      expect(reducer(dirty, resetUserScopedState()).mascotVoices).toEqual({});
    });

    const rehydrate = (key: string, payload?: unknown) => ({ type: REHYDRATE, key, payload });

    it('REHYDRATE keeps only valid entries and tolerates a missing/garbage map', () => {
      const restored = reducer(
        undefined,
        rehydrate('mascot', { mascotVoices: { toshi: 'v-1', bad: '   ', '  ': 'v-2', ok: 'v-3' } })
      );
      expect(restored.mascotVoices).toEqual({ toshi: 'v-1', ok: 'v-3' });
      expect(reducer(undefined, rehydrate('mascot', { color: 'navy' })).mascotVoices).toEqual({});
      expect(
        reducer(undefined, rehydrate('mascot', { mascotVoices: 'not-an-object' })).mascotVoices
      ).toEqual({});
    });
  });

  describe('dual-mascot resolution', () => {
    it('selectDualMascotEnabled is false with one or duplicate mascots, true with two distinct', () => {
      const one = reducer(undefined, setSelectedMascotId('tiny-mascot'));
      expect(selectDualMascotEnabled({ mascot: one })).toBe(false);
      const dup = reducer(one, setSecondaryMascotId('tiny-mascot'));
      expect(selectDualMascotEnabled({ mascot: dup })).toBe(false);
      const two = reducer(one, setSecondaryMascotId('toshi'));
      expect(selectDualMascotEnabled({ mascot: two })).toBe(true);
    });

    it('selectMeetingMascotVoicePair returns null secondary when single', () => {
      let state = reducer(undefined, setSelectedMascotId('tiny-mascot'));
      state = reducer(state, setMascotVoiceId('v-primary'));
      const pair = selectMeetingMascotVoicePair({ mascot: state });
      expect(pair.secondary).toBeNull();
      expect(pair.primary.mascotId).toBe('tiny-mascot');
      expect(pair.primary.voiceId).toBe('v-primary');
    });

    it('selectMeetingMascotVoicePair resolves each slot to its per-mascot voice', () => {
      let state = reducer(undefined, setSelectedMascotId('tiny-mascot'));
      state = reducer(state, setSecondaryMascotId('toshi'));
      state = reducer(state, setMascotVoice({ mascotId: 'tiny-mascot', voiceId: 'v-tiny' }));
      state = reducer(state, setMascotVoice({ mascotId: 'toshi', voiceId: 'v-toshi' }));
      const pair = selectMeetingMascotVoicePair({ mascot: state });
      expect(pair.primary).toEqual({ mascotId: 'tiny-mascot', voiceId: 'v-tiny' });
      expect(pair.secondary).toEqual({ mascotId: 'toshi', voiceId: 'v-toshi' });
    });

    it('per-mascot voice falls back to the effective single voice when unset', () => {
      let state = reducer(undefined, setSelectedMascotId('tiny-mascot'));
      state = reducer(state, setSecondaryMascotId('toshi'));
      state = reducer(state, setMascotVoiceId('v-effective'));
      const pair = selectMeetingMascotVoicePair({ mascot: state });
      // Neither mascot has an explicit override → both use the effective voice.
      expect(pair.primary.voiceId).toBe('v-effective');
      expect(pair.secondary?.voiceId).toBe('v-effective');
    });

    it('tolerates a legacy mascot state missing mascotVoices without throwing', () => {
      // A pre-migration persisted blob or a partial preloadedState can omit
      // `mascotVoices`; the meeting selectors must default it, not crash on
      // `mascotVoices[selectedMascotId]`.
      const legacy = {
        mascot: { selectedMascotId: 'yellow', secondaryMascotId: null },
      } as unknown as Parameters<typeof selectMeetingMascotVoicePair>[0];
      expect(() => selectMeetingMascotVoicePair(legacy)).not.toThrow();
      expect(selectMeetingMascotVoicePair(legacy).primary.mascotId).toBe('yellow');
      expect(selectMascotVoiceFor('yellow')(legacy)).toBeNull();
      expect(selectMascotVoices(legacy)).toEqual({});
    });
  });
});
