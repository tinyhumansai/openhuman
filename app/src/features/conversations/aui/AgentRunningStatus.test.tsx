import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import { AssistantUiRuntimeProvider } from '../../../providers/AssistantUiRuntimeProvider';
import chatRuntimeReducer from '../../../store/chatRuntimeSlice';
import threadReducer from '../../../store/threadSlice';
import {
  AgentRunningStatus,
  hasPendingClarification,
  turnHasTasks,
  waitingOnUser,
} from './AgentRunningStatus';

vi.mock('../../../services/api/threadApi', () => ({
  threadApi: {
    getDerivedTranscript: vi
      .fn()
      .mockResolvedValue({
        threadId: 't-status',
        items: [],
        total: 0,
        hasMore: false,
        hasTranscript: false,
      }),
  },
}));

const THREAD_ID = 't-status';

function buildStore(chatRuntime?: Record<string, unknown>) {
  const reducer = combineReducers({ thread: threadReducer, chatRuntime: chatRuntimeReducer });
  const runtime = chatRuntime
    ? { ...(reducer(undefined, { type: '@@init' }).chatRuntime as object), ...chatRuntime }
    : undefined;
  return configureStore({
    reducer,
    preloadedState: {
      ...(runtime ? { chatRuntime: runtime } : {}),
      thread: {
        threads: [],
        selectedThreadId: THREAD_ID,
        activeThreadIds: {},
        welcomeThreadId: null,
        messagesByThreadId: { [THREAD_ID]: [] },
        messages: [],
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
    } as never,
  });
}

describe('AgentRunningStatus', () => {
  it('falls back to the thinking indicator when assistant-ui has no tasks', () => {
    render(
      <Provider store={buildStore()}>
        <AssistantUiRuntimeProvider>
          <AgentRunningStatus />
        </AssistantUiRuntimeProvider>
      </Provider>
    );

    expect(screen.getByTestId('agent-running-status-thinking')).toBeInTheDocument();
    expect(screen.getByTestId('agent-running-status-thinking')).toHaveAttribute(
      'data-slot',
      'generation-loader'
    );
    expect(screen.getByTestId('agent-running-status-thinking')).toHaveClass('[&>div>span]:size-1');
    expect(screen.queryByTestId('agent-running-status-tasks')).not.toBeInTheDocument();
  });
});

function renderStatus(chatRuntime?: Record<string, unknown>) {
  return render(
    <Provider store={buildStore(chatRuntime)}>
      <AssistantUiRuntimeProvider>
        <AgentRunningStatus />
      </AssistantUiRuntimeProvider>
    </Provider>
  );
}

describe('AgentRunningStatus waiting states', () => {
  it('says it is waiting for approval instead of thinking', () => {
    renderStatus({
      pendingApprovalByThread: {
        [THREAD_ID]: { requestId: 'r1', toolName: 'shell', message: 'run', expiresAt: null },
      },
    });
    const line = screen.getByTestId('agent-running-status-waiting');
    expect(line).toHaveAttribute('data-waiting', 'approval');
    expect(line).toHaveTextContent('Waiting for your approval');
    expect(screen.queryByTestId('agent-running-status-thinking')).not.toBeInTheDocument();
  });

  it('says it is waiting for a plan review', () => {
    renderStatus({ pendingPlanReviewByThread: { [THREAD_ID]: { requestId: 'p1' } } });
    expect(screen.getByTestId('agent-running-status-waiting')).toHaveAttribute(
      'data-waiting',
      'review'
    );
  });
});

describe('AgentRunningStatus while compacting', () => {
  it('says it is summarizing earlier conversation instead of thinking', () => {
    renderStatus({
      inferenceStatusByThread: {
        [THREAD_ID]: { phase: 'compacting', iteration: 1, maxIterations: 0 },
      },
    });
    expect(screen.getByTestId('agent-running-status-thinking')).toHaveTextContent(
      'Summarizing earlier conversation'
    );
  });

  it('counts the elapsed time once a second has passed', () => {
    vi.useFakeTimers();
    try {
      renderStatus({
        inferenceStatusByThread: {
          [THREAD_ID]: { phase: 'compacting', iteration: 1, maxIterations: 0 },
        },
      });
      act(() => {
        vi.advanceTimersByTime(2500);
      });
      expect(screen.getByTestId('agent-running-status-thinking')).toHaveTextContent(
        /Summarizing earlier conversation · \d/
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it('thinks again once the next model call starts', () => {
    renderStatus({
      inferenceStatusByThread: {
        [THREAD_ID]: { phase: 'thinking', iteration: 1, maxIterations: 0 },
      },
    });
    expect(screen.getByTestId('agent-running-status-thinking')).not.toHaveTextContent(
      'Summarizing'
    );
  });
});

describe('waitingOnUser', () => {
  it('ranks approval over review over answer', () => {
    expect(waitingOnUser({ approval: true, review: true, answer: true })).toBe('approval');
    expect(waitingOnUser({ approval: false, review: true, answer: true })).toBe('review');
    expect(waitingOnUser({ approval: false, review: false, answer: true })).toBe('answer');
    expect(waitingOnUser({ approval: false, review: false, answer: false })).toBeNull();
  });
});

describe('hasPendingClarification', () => {
  const ask = (result?: unknown) => ({
    type: 'tool-call',
    toolName: 'ask_user_clarification',
    result,
  });

  it('is true only for an unanswered question on the newest assistant message', () => {
    expect(hasPendingClarification([{ role: 'assistant', content: [ask()] }])).toBe(true);
    expect(hasPendingClarification([{ role: 'assistant', content: [ask('ok')] }])).toBe(false);
    expect(
      hasPendingClarification([
        { role: 'assistant', content: [ask()] },
        { role: 'assistant', content: [{ type: 'text' }] },
      ])
    ).toBe(false);
    expect(
      hasPendingClarification([
        { role: 'assistant', content: [ask()] },
        { role: 'user', content: [{ type: 'text' }] },
      ])
    ).toBe(true);
    expect(hasPendingClarification([])).toBe(false);
  });
});

describe('turnHasTasks', () => {
  const user = { id: 'u-2', role: 'user' };
  const earlier = { id: 'a-1', role: 'assistant' };
  const running = { id: 'a-2', role: 'assistant' };

  it('ignores delegations from earlier turns', () => {
    expect(turnHasTasks([{ messageId: 'a-1' }], [earlier, user, running])).toBe(false);
  });

  it('is true when the running turn delegated work', () => {
    expect(
      turnHasTasks([{ messageId: 'a-1' }, { messageId: 'a-2' }], [earlier, user, running])
    ).toBe(true);
  });

  it('is false before the running turn has a reply', () => {
    expect(turnHasTasks([{ messageId: 'a-1' }], [earlier, user])).toBe(false);
  });
});
