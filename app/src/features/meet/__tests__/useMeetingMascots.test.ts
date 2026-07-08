import { renderHook } from '@testing-library/react';
import { createElement, type PropsWithChildren } from 'react';
import { Provider } from 'react-redux';
import { describe, expect, it } from 'vitest';

import { createTestStore } from '../../../test/test-utils';
import {
  type MeetingPhase,
  useMeetingMascots,
  type UseMeetingMascotsInput,
} from '../useMeetingMascots';

/**
 * Seed the mascot slice via preloadedState. `secondaryMascotId` distinct from
 * `selectedMascotId` is what flips `selectDualMascotEnabled` on.
 */
function makeWrapper(mascot: Record<string, unknown>) {
  const store = createTestStore({
    mascot: {
      color: 'yellow',
      voiceId: null,
      voiceGender: 'male',
      voiceUseLocaleDefault: false,
      selectedMascotId: null,
      secondaryMascotId: null,
      mascotVoices: {},
      customMascotGifUrl: null,
      customPrimaryColor: '#F7D145',
      customSecondaryColor: '#B23C05',
      ...mascot,
    },
  });
  return function Wrapper({ children }: PropsWithChildren) {
    return createElement(Provider, { store, children });
  };
}

function run(mascot: Record<string, unknown>, input: UseMeetingMascotsInput) {
  const { result } = renderHook(() => useMeetingMascots(input), { wrapper: makeWrapper(mascot) });
  return result.current;
}

const SINGLE = { selectedMascotId: 'tiny-mascot', secondaryMascotId: null };
const DUAL = { selectedMascotId: 'tiny-mascot', secondaryMascotId: 'toshi' };

describe('useMeetingMascots — dualEnabled gating', () => {
  it('is single when no secondary mascot is set', () => {
    const state = run(SINGLE, { speaking: false, activeMascotSlot: 0, phase: 'active' });
    expect(state.dualEnabled).toBe(false);
    expect(state.secondary).toBeNull();
    expect(state.primary.mascotId).toBe('tiny-mascot');
  });

  it('is single when the secondary equals the primary (same mascot picked twice)', () => {
    const state = run(
      { selectedMascotId: 'tiny-mascot', secondaryMascotId: 'tiny-mascot' },
      { speaking: false, activeMascotSlot: 0, phase: 'active' }
    );
    expect(state.dualEnabled).toBe(false);
    expect(state.secondary).toBeNull();
  });

  it('is dual when a distinct secondary mascot is set', () => {
    const state = run(DUAL, { speaking: false, activeMascotSlot: 0, phase: 'active' });
    expect(state.dualEnabled).toBe(true);
    expect(state.primary.mascotId).toBe('tiny-mascot');
    expect(state.secondary?.mascotId).toBe('toshi');
  });
});

describe('useMeetingMascots — single-mascot face (preserves original behavior)', () => {
  it('primary follows speaking → speaking, else idle; secondary null', () => {
    const speaking = run(SINGLE, { speaking: true, activeMascotSlot: 0, phase: 'active' });
    expect(speaking.primary.face).toBe('speaking');
    expect(speaking.secondary).toBeNull();

    const silent = run(SINGLE, { speaking: false, activeMascotSlot: 0, phase: 'active' });
    expect(silent.primary.face).toBe('idle');
  });

  it('single-mascot ignores phase for the face (no greeting/signoff wave)', () => {
    // The single path deliberately keeps the legacy speaking/idle mapping.
    for (const phase of ['greeting', 'active', 'signoff'] as MeetingPhase[]) {
      const state = run(SINGLE, { speaking: false, activeMascotSlot: 0, phase });
      expect(state.primary.face).toBe('idle');
    }
  });
});

describe('useMeetingMascots — dual face table', () => {
  it('greeting → both slots wave, regardless of speaking/activeSlot', () => {
    for (const activeMascotSlot of [0, 1] as const) {
      for (const speaking of [false, true]) {
        const state = run(DUAL, { speaking, activeMascotSlot, phase: 'greeting' });
        expect(state.primary.face).toBe('waving');
        expect(state.secondary?.face).toBe('waving');
      }
    }
  });

  it('signoff → both slots wave, regardless of speaking/activeSlot', () => {
    for (const activeMascotSlot of [0, 1] as const) {
      for (const speaking of [false, true]) {
        const state = run(DUAL, { speaking, activeMascotSlot, phase: 'signoff' });
        expect(state.primary.face).toBe('waving');
        expect(state.secondary?.face).toBe('waving');
      }
    }
  });

  describe('active phase — activeSlot × speaking', () => {
    it('slot 0 active + speaking → primary speaking, secondary thinking', () => {
      const s = run(DUAL, { speaking: true, activeMascotSlot: 0, phase: 'active' });
      expect(s.primary.face).toBe('speaking');
      expect(s.secondary?.face).toBe('thinking');
    });

    it('slot 0 active + not speaking → primary listening, secondary thinking', () => {
      const s = run(DUAL, { speaking: false, activeMascotSlot: 0, phase: 'active' });
      expect(s.primary.face).toBe('listening');
      expect(s.secondary?.face).toBe('thinking');
    });

    it('slot 1 active + speaking → secondary speaking, primary thinking', () => {
      const s = run(DUAL, { speaking: true, activeMascotSlot: 1, phase: 'active' });
      expect(s.secondary?.face).toBe('speaking');
      expect(s.primary.face).toBe('thinking');
    });

    it('slot 1 active + not speaking → secondary listening, primary thinking', () => {
      const s = run(DUAL, { speaking: false, activeMascotSlot: 1, phase: 'active' });
      expect(s.secondary?.face).toBe('listening');
      expect(s.primary.face).toBe('thinking');
    });
  });
});
