import { describe, expect, it } from 'vitest';

import backendMeetReducer, {
  resetBackendMeet,
  setBackendMeetError,
  setBackendMeetHarness,
  setBackendMeetJoined,
  setBackendMeetJoining,
  setBackendMeetLeft,
  setBackendMeetReply,
  setBackendMeetTranscript,
} from './backendMeetSlice';

const initial = backendMeetReducer(undefined, { type: 'init' });

describe('backendMeetSlice', () => {
  it('starts in idle state', () => {
    expect(initial.status).toBe('idle');
    expect(initial.meetUrl).toBeNull();
    expect(initial.lastReply).toBeNull();
    expect(initial.transcript).toBeNull();
  });

  it('transitions to joining', () => {
    const state = backendMeetReducer(
      initial,
      setBackendMeetJoining({ meetUrl: 'https://meet.google.com/abc-defg-hij' })
    );
    expect(state.status).toBe('joining');
    expect(state.meetUrl).toBe('https://meet.google.com/abc-defg-hij');
  });

  it('transitions to active on joined', () => {
    const joining = backendMeetReducer(
      initial,
      setBackendMeetJoining({ meetUrl: 'https://meet.google.com/abc-defg-hij' })
    );
    const state = backendMeetReducer(
      joining,
      setBackendMeetJoined({ meetUrl: 'https://meet.google.com/abc-defg-hij' })
    );
    expect(state.status).toBe('active');
  });

  it('transitions to ended on left', () => {
    const active = backendMeetReducer(
      backendMeetReducer(initial, setBackendMeetJoined({ meetUrl: 'x' })),
      setBackendMeetLeft({ reason: 'call-ended' })
    );
    expect(active.status).toBe('ended');
  });

  it('stores reply events', () => {
    const state = backendMeetReducer(
      initial,
      setBackendMeetReply({ transcript: 'Hey bot', reply: 'Hello!', emotion: 'happy' })
    );
    expect(state.lastReply).toEqual({ transcript: 'Hey bot', reply: 'Hello!', emotion: 'happy' });
  });

  it('stores harness events', () => {
    const state = backendMeetReducer(
      initial,
      setBackendMeetHarness({
        transcript: 'Check my email',
        instruction: 'read 5 latest emails',
        emotion: 'thinking',
      })
    );
    expect(state.lastHarness?.instruction).toBe('read 5 latest emails');
  });

  it('stores transcript on close', () => {
    const state = backendMeetReducer(
      initial,
      setBackendMeetTranscript({
        turns: [
          { role: 'user', content: 'Hello' },
          { role: 'assistant', content: 'Hi there!' },
        ],
        duration_ms: 120000,
      })
    );
    expect(state.transcript?.turns).toHaveLength(2);
    expect(state.transcript?.duration_ms).toBe(120000);
  });

  it('stores error', () => {
    const state = backendMeetReducer(initial, setBackendMeetError({ error: 'connection failed' }));
    expect(state.status).toBe('error');
    expect(state.error).toBe('connection failed');
  });

  it('resets to initial state', () => {
    const active = backendMeetReducer(initial, setBackendMeetJoined({ meetUrl: 'x' }));
    const state = backendMeetReducer(active, resetBackendMeet());
    expect(state).toEqual(initial);
  });
});
