import { describe, expect, it } from 'vitest';

import backendMeetReducer, {
  appendBackendMeetTranscriptDelta,
  resetBackendMeet,
  selectBackendMeetLiveTranscript,
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

  describe('live transcript (transcript_delta, #4304)', () => {
    it('appends sequential delta turns to the live buffer', () => {
      let state = backendMeetReducer(
        initial,
        appendBackendMeetTranscriptDelta({
          turn: { role: 'user', content: 'Hello' },
          index: 0,
          is_partial: false,
        })
      );
      state = backendMeetReducer(
        state,
        appendBackendMeetTranscriptDelta({
          turn: { role: 'assistant', content: 'Hi there' },
          index: 1,
          is_partial: false,
        })
      );
      expect(state.liveTranscript).toEqual([
        { role: 'user', content: 'Hello' },
        { role: 'assistant', content: 'Hi there' },
      ]);
      expect(state.livePartialIndex).toBeNull();
    });

    it('marks a partial line and supersedes it when finalized at the same index', () => {
      let state = backendMeetReducer(
        initial,
        appendBackendMeetTranscriptDelta({
          turn: { role: 'user', content: 'Hel' },
          index: 0,
          is_partial: true,
        })
      );
      expect(state.livePartialIndex).toBe(0);
      expect(state.liveTranscript[0]?.content).toBe('Hel');

      // Final delta at the same index replaces the partial and clears the flag.
      state = backendMeetReducer(
        state,
        appendBackendMeetTranscriptDelta({
          turn: { role: 'user', content: 'Hello there' },
          index: 0,
          is_partial: false,
        })
      );
      expect(state.liveTranscript).toHaveLength(1);
      expect(state.liveTranscript[0]?.content).toBe('Hello there');
      expect(state.livePartialIndex).toBeNull();
    });

    it('keys by backend index: gaps (skipped [System] turns) do not break supersede', () => {
      // index 0 finalized, then a partial preview lands at index 2 (index 1 is a
      // skipped [System] turn never sent as a delta), then index 2 is finalized.
      let state = backendMeetReducer(
        initial,
        appendBackendMeetTranscriptDelta({
          turn: { role: 'user', content: '[Alice] hi' },
          index: 0,
          is_partial: false,
        })
      );
      state = backendMeetReducer(
        state,
        appendBackendMeetTranscriptDelta({
          turn: { role: 'user', content: '[Bob] in pro' },
          index: 2,
          is_partial: true,
        })
      );
      expect(state.livePartialIndex).toBe(2);
      // Finalize at the SAME backend index → replaces the partial in place, no dup.
      state = backendMeetReducer(
        state,
        appendBackendMeetTranscriptDelta({
          turn: { role: 'user', content: '[Bob] in progress' },
          index: 2,
          is_partial: false,
        })
      );
      expect(state.livePartialIndex).toBeNull();
      expect(state.liveTranscript[0]?.content).toBe('[Alice] hi');
      expect(state.liveTranscript[2]?.content).toBe('[Bob] in progress');
      // The gap at index 1 stays empty; no duplicate Bob turn was appended.
      expect(state.liveTranscript[1]).toBeUndefined();
      const populated = state.liveTranscript.filter(Boolean);
      expect(populated).toHaveLength(2);
    });

    it('selector returns an empty array when the buffer is absent (legacy state)', () => {
      // A store shaped before this slice field existed has no liveTranscript;
      // the selector must not hand back undefined (would crash the panel).
      const legacy = { backendMeet: { status: 'active' } } as never;
      expect(selectBackendMeetLiveTranscript(legacy)).toEqual([]);
    });

    it('ignores a delta with a negative index', () => {
      const state = backendMeetReducer(
        initial,
        appendBackendMeetTranscriptDelta({
          turn: { role: 'user', content: 'bad' },
          index: -1,
          is_partial: false,
        })
      );
      expect(state.liveTranscript.filter(Boolean)).toHaveLength(0);
      expect(state.livePartialIndex).toBeNull();
    });

    it('clears the live buffer on join', () => {
      const withLive = backendMeetReducer(
        initial,
        appendBackendMeetTranscriptDelta({
          turn: { role: 'user', content: 'stale' },
          index: 0,
          is_partial: false,
        })
      );
      const joining = backendMeetReducer(
        withLive,
        setBackendMeetJoining({ meetUrl: 'https://meet.google.com/abc-defg-hij' })
      );
      expect(joining.liveTranscript).toEqual([]);
      expect(joining.livePartialIndex).toBeNull();
    });

    it('clears the live buffer on leave', () => {
      const withLive = backendMeetReducer(
        initial,
        appendBackendMeetTranscriptDelta({
          turn: { role: 'user', content: 'mid-call' },
          index: 0,
          is_partial: true,
        })
      );
      const left = backendMeetReducer(withLive, setBackendMeetLeft({ reason: 'call-ended' }));
      expect(left.liveTranscript).toEqual([]);
      expect(left.livePartialIndex).toBeNull();
    });

    it('reconciles: final transcript empties the live buffer', () => {
      const withLive = backendMeetReducer(
        initial,
        appendBackendMeetTranscriptDelta({
          turn: { role: 'user', content: 'Hello' },
          index: 0,
          is_partial: false,
        })
      );
      const reconciled = backendMeetReducer(
        withLive,
        setBackendMeetTranscript({ turns: [{ role: 'user', content: 'Hello' }], duration_ms: 1000 })
      );
      expect(reconciled.transcript?.turns).toHaveLength(1);
      expect(reconciled.liveTranscript).toEqual([]);
      expect(reconciled.livePartialIndex).toBeNull();
    });
  });
});
