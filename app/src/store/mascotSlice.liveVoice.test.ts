import { REHYDRATE } from 'redux-persist';
import { describe, expect, it } from 'vitest';

import mascotReducer, {
  selectChatMascotLiveVoicePhase,
  setChatMascotLiveVoicePhase,
} from './mascotSlice';

describe('mascotSlice — live voice phase', () => {
  it('defaults to off', () => {
    const state = mascotReducer(undefined, { type: '@@init' });
    expect(selectChatMascotLiveVoicePhase({ mascot: state })).toBe('off');
  });

  it('tracks the phase and coerces unknown values to off', () => {
    let state = mascotReducer(undefined, setChatMascotLiveVoicePhase('speaking'));
    expect(state.chatMascotLiveVoicePhase).toBe('speaking');
    const same = mascotReducer(state, setChatMascotLiveVoicePhase('speaking'));
    expect(same).toBe(state);
    state = mascotReducer(state, setChatMascotLiveVoicePhase('bogus' as never));
    expect(state.chatMascotLiveVoicePhase).toBe('off');
  });

  it('is never restored from a persisted blob', () => {
    const live = mascotReducer(undefined, setChatMascotLiveVoicePhase('listening'));
    const rehydrated = mascotReducer(live, {
      type: REHYDRATE,
      key: 'mascot',
      payload: { chatMascotLiveVoicePhase: 'speaking' },
    });
    expect(rehydrated.chatMascotLiveVoicePhase).toBe('off');
  });
});
