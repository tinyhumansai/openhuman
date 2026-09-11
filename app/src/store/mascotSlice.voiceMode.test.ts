import { REHYDRATE } from 'redux-persist';
import { describe, expect, it } from 'vitest';

import mascotReducer, { selectVoiceMode, setVoiceMode } from './mascotSlice';

type Action = Parameters<typeof mascotReducer>[1];
const rehydrate = (payload: unknown): Action =>
  ({ type: REHYDRATE, key: 'mascot', payload }) as unknown as Action;

describe('mascotSlice voiceMode (#5399)', () => {
  it('defaults to classic', () => {
    const state = mascotReducer(undefined, { type: '@@INIT' } as Action);
    expect(state.voiceMode).toBe('classic');
    expect(selectVoiceMode({ mascot: state })).toBe('classic');
  });

  it('setVoiceMode toggles between realtime and classic', () => {
    let state = mascotReducer(undefined, setVoiceMode('realtime'));
    expect(state.voiceMode).toBe('realtime');
    state = mascotReducer(state, setVoiceMode('classic'));
    expect(state.voiceMode).toBe('classic');
  });

  it('setVoiceMode ignores an out-of-range value', () => {
    const start = mascotReducer(undefined, setVoiceMode('realtime'));
    const next = mascotReducer(start, setVoiceMode('garbage' as never));
    expect(next.voiceMode).toBe('realtime');
  });

  it('rehydrate restores a valid persisted voiceMode', () => {
    const state = mascotReducer(undefined, rehydrate({ voiceMode: 'realtime' }));
    expect(state.voiceMode).toBe('realtime');
  });

  it('rehydrate falls back to classic for a missing or corrupt voiceMode', () => {
    expect(mascotReducer(undefined, rehydrate({})).voiceMode).toBe('classic');
    expect(mascotReducer(undefined, rehydrate({ voiceMode: 'nope' })).voiceMode).toBe('classic');
  });
});
