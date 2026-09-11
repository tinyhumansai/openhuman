/**
 * A redelivered `subagent_spawned` must not cancel a live sub-agent question.
 *
 * `continue_subagent` announces a resume by republishing `subagent_spawned` for
 * the same task/agent, so "a row that already exists got spawned again" is the
 * only resume signal the frontend receives — and it is exactly what Socket.IO
 * redelivering the ORIGINAL spawn looks like. This socket reconnects and
 * replays freely (13+ times in one measured session), so the failure is not
 * hypothetical: the pause is cleared, the question disappears, and the user is
 * left with a spinner for a child that is still blocked on them.
 *
 * Two independent guards, tested here at the seam they defend: the provider's
 * event-seen cache keyed on the core's `(request_id, seq)` stamp, and — for the
 * case that cache has evicted — the reducer's own identity check.
 */
import { render } from '@testing-library/react';
import { act } from 'react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import * as chatService from '../../services/chatService';
import { threadApi } from '../../services/api/threadApi';
import { store } from '../../store';
import { clearAllChatRuntime, resetSessionTokenUsage } from '../../store/chatRuntimeSlice';
import { setStatusForUser } from '../../store/socketSlice';
import { clearAllThreads } from '../../store/threadSlice';
import ChatRuntimeProvider from '../ChatRuntimeProvider';

vi.mock('../../services/chatService', async () => {
  const actual = await vi.importActual<typeof chatService>('../../services/chatService');
  return { ...actual, subscribeChatEvents: vi.fn() };
});

vi.mock('../../services/api/threadApi', () => ({
  threadApi: {
    createNewThread: vi.fn(),
    getThreads: vi.fn(),
    getThreadMessages: vi.fn(),
    appendMessage: vi.fn(),
    generateTitleIfNeeded: vi.fn(),
    updateMessage: vi.fn(),
    deleteThread: vi.fn(),
    purge: vi.fn(),
    getTaskBoard: vi.fn(),
    putTaskBoard: vi.fn(),
  },
}));

vi.mock('../../hooks/usageRefresh', () => ({ requestUsageRefresh: vi.fn() }));
vi.mock('../../hooks/useRefetchSnapshotOnTurnEnd', () => ({
  useRefetchSnapshotOnTurnEnd: () => ({ refetch: vi.fn() }),
}));

const THREAD = 't-replay';
const ROW = `${THREAD}:subagent:sub-1:researcher`;
const QUESTION = 'Which of the two repos should I patch?';

function renderProvider(): chatService.ChatEventListeners {
  let captured: chatService.ChatEventListeners = {};
  vi.mocked(chatService.subscribeChatEvents).mockImplementation(listeners => {
    captured = listeners;
    return () => {};
  });
  store.dispatch(setStatusForUser({ userId: '__pending__', status: 'connected' }));
  render(
    <Provider store={store}>
      <ChatRuntimeProvider>
        <div />
      </ChatRuntimeProvider>
    </Provider>
  );
  return captured;
}

/** The socket frame the core emits, `seq` and all. */
function spawnEvent(seq: number, requestId = 'req-1') {
  return {
    thread_id: THREAD,
    request_id: requestId,
    round: 0,
    tool_name: 'researcher',
    skill_id: 'sub-1',
    message: "Sub-agent 'researcher' spawned",
    seq,
    subagent: { mode: 'typed' },
  };
}

function awaitingEvent(seq: number, requestId = 'req-1') {
  return {
    thread_id: THREAD,
    request_id: requestId,
    round: 0,
    tool_name: 'researcher',
    skill_id: 'sub-1',
    message: QUESTION,
    success: true,
    seq,
  };
}

function row() {
  return store.getState().chatRuntime.toolTimelineByThread[THREAD]?.find(e => e.id === ROW);
}

describe('ChatRuntimeProvider — replayed sub-agent spawn', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    store.dispatch(clearAllThreads());
    store.dispatch(clearAllChatRuntime());
    store.dispatch(resetSessionTokenUsage());
    store.dispatch(setStatusForUser({ userId: '__pending__', status: 'disconnected' }));
    vi.mocked(threadApi.getThreads).mockResolvedValue({ threads: [], count: 0 });
  });

  it('keeps the question when the original spawn frame is redelivered', () => {
    const listeners = renderProvider();

    act(() => {
      listeners.onSubagentSpawned?.(spawnEvent(3));
      listeners.onSubagentAwaitingUser?.(awaitingEvent(7));
    });
    expect(row()?.status).toBe('awaiting_user');
    expect(row()?.subagent?.awaitingQuestion).toBe(QUESTION);

    act(() => {
      // Reconnect. The core does not re-emit; the transport re-delivers the
      // same frame, `seq` included.
      listeners.onSubagentSpawned?.(spawnEvent(3));
    });

    expect(row()?.status).toBe('awaiting_user');
    expect(row()?.subagent?.status).toBe('awaiting_user');
    expect(row()?.subagent?.awaitingQuestion).toBe(QUESTION);
  });

  it('still resumes on a real continue_subagent spawn', () => {
    const listeners = renderProvider();

    act(() => {
      listeners.onSubagentSpawned?.(spawnEvent(3));
      listeners.onSubagentAwaitingUser?.(awaitingEvent(7));
    });
    expect(row()?.status).toBe('awaiting_user');

    act(() => {
      // The user answered, so this is a new parent turn: new request, and the
      // bridge's per-request counter starts over.
      listeners.onSubagentSpawned?.(spawnEvent(0, 'req-2'));
    });

    expect(row()?.status).toBe('running');
    expect(row()?.subagent?.awaitingQuestion).toBeUndefined();
  });

  it('does not re-park a resumed child when the awaiting frame is redelivered', () => {
    const listeners = renderProvider();

    act(() => {
      listeners.onSubagentSpawned?.(spawnEvent(3));
      listeners.onSubagentAwaitingUser?.(awaitingEvent(7));
      listeners.onSubagentSpawned?.(spawnEvent(0, 'req-2'));
    });
    expect(row()?.status).toBe('running');

    act(() => {
      listeners.onSubagentAwaitingUser?.(awaitingEvent(7));
    });

    expect(row()?.status).toBe('running');
  });
});
