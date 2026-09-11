import { REHYDRATE } from 'redux-persist';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import reducer, {
  LEGACY_SPEAK_REPLIES_KEY,
  migrateLegacySpeakReplies,
  selectChatMascotDismissed,
  selectChatMascotExpanded,
  selectChatMascotListening,
  selectSpeakReplies,
  setChatMascotDismissed,
  setChatMascotExpanded,
  setChatMascotListening,
  setSpeakReplies,
} from '../mascotSlice';

const rehydrate = (key: string, payload?: unknown) => ({ type: REHYDRATE, key, payload });

describe('mascotSlice — chat mascot stage', () => {
  beforeEach(() => window.localStorage.clear());
  afterEach(() => window.localStorage.clear());

  it('starts docked, speaking, and not listening', () => {
    const state = reducer(undefined, { type: '@@INIT' });
    expect(selectChatMascotExpanded({ mascot: state })).toBe(false);
    expect(selectSpeakReplies({ mascot: state })).toBe(true);
    expect(selectChatMascotListening({ mascot: state })).toBe(false);
  });

  it('opens and closes the stage', () => {
    const open = reducer(undefined, setChatMascotExpanded(true));
    expect(open.chatMascotExpanded).toBe(true);
    expect(reducer(open, setChatMascotExpanded(false)).chatMascotExpanded).toBe(false);
  });

  it('keeps the same state object when setting the value it already has', () => {
    // The overlay + stage column both subscribe; a no-op dispatch that produced
    // a new reference would re-render them (and restart the travel) for nothing.
    const open = reducer(undefined, setChatMascotExpanded(true));
    expect(reducer(open, setChatMascotExpanded(true))).toBe(open);
  });

  it('records a hot mic, ignoring repeats', () => {
    const listening = reducer(undefined, setChatMascotListening(true));
    expect(listening.chatMascotListening).toBe(true);
    expect(reducer(listening, setChatMascotListening(true))).toBe(listening);
    expect(reducer(listening, setChatMascotListening(false)).chatMascotListening).toBe(false);
  });

  describe('dismissal', () => {
    it('starts visible', () => {
      expect(selectChatMascotDismissed({ mascot: reducer(undefined, { type: '@@INIT' }) })).toBe(
        false
      );
    });

    it('collapses the voice stage when dismissed', () => {
      // Otherwise the two flags disagree, and restoring the mascot would pop its
      // stage open again without the user asking.
      const expanded = reducer(undefined, setChatMascotExpanded(true));
      const dismissed = reducer(expanded, setChatMascotDismissed(true));
      expect(dismissed.chatMascotDismissed).toBe(true);
      expect(dismissed.chatMascotExpanded).toBe(false);
    });

    it('restores docked, not expanded', () => {
      let state = reducer(undefined, setChatMascotExpanded(true));
      state = reducer(state, setChatMascotDismissed(true));
      state = reducer(state, setChatMascotDismissed(false));
      expect(state.chatMascotDismissed).toBe(false);
      expect(state.chatMascotExpanded).toBe(false);
    });

    it('ignores a repeat dismissal', () => {
      const dismissed = reducer(undefined, setChatMascotDismissed(true));
      expect(reducer(dismissed, setChatMascotDismissed(true))).toBe(dismissed);
    });

    it('survives a reload', () => {
      const state = reducer(undefined, rehydrate('mascot', { chatMascotDismissed: true }));
      expect(state.chatMascotDismissed).toBe(true);
    });

    it('defaults to visible for a blob that predates the setting', () => {
      const state = reducer(undefined, rehydrate('mascot', { color: 'navy' }));
      expect(state.chatMascotDismissed).toBe(false);
    });
  });

  it('sets the speak-replies preference', () => {
    expect(reducer(undefined, setSpeakReplies(false)).speakReplies).toBe(false);
  });

  describe('REHYDRATE', () => {
    it('restores the persisted stage + speech preferences', () => {
      const state = reducer(
        undefined,
        rehydrate('mascot', { chatMascotExpanded: true, speakReplies: false })
      );
      expect(state.chatMascotExpanded).toBe(true);
      expect(state.speakReplies).toBe(false);
    });

    it('defaults a pre-merge blob to docked + speaking', () => {
      const state = reducer(undefined, rehydrate('mascot', { color: 'navy' }));
      expect(state.chatMascotExpanded).toBe(false);
      expect(state.speakReplies).toBe(true);
    });

    it('never restores a listening mic', () => {
      // Transient hardware state: a restored `true` would pin the mascot into a
      // listening pose with no mic running.
      const listening = reducer(undefined, setChatMascotListening(true));
      const state = reducer(listening, rehydrate('mascot', { chatMascotListening: true }));
      expect(state.chatMascotListening).toBe(false);
    });

    it('takes the migrated value straight from the payload', () => {
      const state = reducer(undefined, rehydrate('mascot', { speakReplies: false }));
      expect(state.speakReplies).toBe(false);
    });

    it('does not touch localStorage — the reducer must stay pure', () => {
      // The migration is a redux-persist `migrate` hook precisely so replaying
      // the action log cannot take a different branch. If this reducer ever
      // reads the legacy key again, this test fails.
      window.localStorage.setItem(LEGACY_SPEAK_REPLIES_KEY, '0');

      const state = reducer(undefined, rehydrate('mascot', {}));

      expect(state.speakReplies).toBe(true); // the slice default, untouched
      expect(window.localStorage.getItem(LEGACY_SPEAK_REPLIES_KEY)).toBe('0');
    });
  });
});

describe('migrateLegacySpeakReplies', () => {
  beforeEach(() => window.localStorage.clear());

  it('passes the blob through untouched when there is nothing to migrate', () => {
    const blob = { color: 'navy' };
    expect(migrateLegacySpeakReplies(blob)).toBe(blob);
    expect(migrateLegacySpeakReplies(undefined)).toBeUndefined();
  });

  it('folds a disabled legacy value into the blob and clears the key', () => {
    // A user who turned TTS off before the merge must not have it silently
    // turned back on by the new `true` default.
    window.localStorage.setItem(LEGACY_SPEAK_REPLIES_KEY, '0');

    expect(migrateLegacySpeakReplies({ color: 'navy' })).toEqual({
      color: 'navy',
      speakReplies: false,
    });
    expect(window.localStorage.getItem(LEGACY_SPEAK_REPLIES_KEY)).toBeNull();
  });

  it('folds an enabled legacy value over a persisted false', () => {
    window.localStorage.setItem(LEGACY_SPEAK_REPLIES_KEY, '1');
    expect(migrateLegacySpeakReplies({ speakReplies: false })).toEqual({ speakReplies: true });
  });

  it('seeds a blob that does not exist yet', () => {
    window.localStorage.setItem(LEGACY_SPEAK_REPLIES_KEY, '1');
    expect(migrateLegacySpeakReplies(undefined)).toEqual({ speakReplies: true });
  });

  it('migrates exactly once', () => {
    window.localStorage.setItem(LEGACY_SPEAK_REPLIES_KEY, '1');
    migrateLegacySpeakReplies({});
    expect(migrateLegacySpeakReplies({})).toEqual({});
  });
});
