import { renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { OrchestrationMessage, SessionSummary } from './orchestrationClient';
import {
  mapTranscriptMessage,
  useContactSessions,
  useSessionTranscript,
} from './useOrchestrationSessions';

const sessionsList = vi.hoisted(() => vi.fn());
const messagesList = vi.hoisted(() => vi.fn());
vi.mock('./orchestrationClient', async orig => ({
  ...(await orig<typeof import('./orchestrationClient')>()),
  orchestrationClient: { sessionsList, messagesList },
}));
vi.mock('../../services/socketService', () => ({ socketService: { on: vi.fn(), off: vi.fn() } }));

const session = (over: Partial<SessionSummary>): SessionSummary => ({
  sessionId: 's1',
  agentId: '@a',
  source: 'claude',
  status: 'idle',
  chatKind: 'session',
  lastMessageAt: '2026-07-08T00:00:00Z',
  unread: 0,
  active: false,
  pinned: false,
  ...over,
});

describe('mapTranscriptMessage', () => {
  it('maps wire fields incl. tool outcome, defaulting from to role/agentId', () => {
    const wire = {
      id: 'm1',
      agentId: '@a',
      sessionId: 's1',
      chatKind: 'session',
      role: '',
      body: 'out',
      timestamp: 't',
      seq: 1,
      eventKind: 'tool_result',
      toolName: 'Bash',
      callId: 'c1',
      ok: false,
      isError: true,
      exitCode: 2,
    } as OrchestrationMessage;
    const m = mapTranscriptMessage(wire);
    expect(m.from).toBe('@a');
    expect(m.eventKind).toBe('tool_result');
    expect(m.ok).toBe(false);
    expect(m.isError).toBe(true);
    expect(m.exitCode).toBe(2);
  });
});

describe('useContactSessions', () => {
  beforeEach(() => vi.clearAllMocks());

  it('groups session-kind rows by contact agent id', async () => {
    sessionsList.mockResolvedValue({
      sessions: [
        session({ sessionId: 's1', agentId: '@a' }),
        session({ sessionId: 's2', agentId: '@a' }),
        session({ sessionId: 's3', agentId: '@b' }),
        session({ sessionId: 'master', agentId: 'master', chatKind: 'master' }),
      ],
    });
    const { result } = renderHook(() => useContactSessions());
    await waitFor(() => expect(result.current.state.status).toBe('ok'));
    expect(result.current.sessions).toHaveLength(3);
    expect(result.current.byContact.get('@a')).toHaveLength(2);
    expect(result.current.byContact.get('@b')).toHaveLength(1);
  });

  it('surfaces an error state', async () => {
    sessionsList.mockRejectedValue(new Error('boom'));
    const { result } = renderHook(() => useContactSessions());
    await waitFor(() => expect(result.current.state.status).toBe('error'));
  });
});

describe('useSessionTranscript', () => {
  beforeEach(() => vi.clearAllMocks());

  it('loads a session transcript', async () => {
    messagesList.mockResolvedValue({
      messages: [
        {
          id: 'm1',
          agentId: '@a',
          sessionId: 's1',
          chatKind: 'session',
          role: 'agent',
          body: 'hi',
          timestamp: 't',
          seq: 1,
        },
      ],
    });
    const { result } = renderHook(() => useSessionTranscript('s1'));
    await waitFor(() => expect(result.current.state.status).toBe('ok'));
    expect(result.current.messages).toHaveLength(1);
    expect(messagesList).toHaveBeenCalledWith({ chat: 's1', limit: 100 });
  });

  it('stays idle for a null session', async () => {
    const { result } = renderHook(() => useSessionTranscript(null));
    await waitFor(() => expect(result.current.state.status).toBe('idle'));
    expect(messagesList).not.toHaveBeenCalled();
  });
});
