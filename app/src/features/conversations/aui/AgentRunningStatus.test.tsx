import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import { AssistantUiRuntimeProvider } from '../../../providers/AssistantUiRuntimeProvider';
import chatRuntimeReducer from '../../../store/chatRuntimeSlice';
import threadReducer from '../../../store/threadSlice';
import { AgentRunningStatus, hasPendingClarification, waitingOnUser } from './AgentRunningStatus';

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

  it('preserves thinking start across unmount and remount on thread switch', () => {
    const { unmount } = render(
      <Provider store={buildStore()}>
        <AssistantUiRuntimeProvider>
          <AgentRunningStatus />
        </AssistantUiRuntimeProvider>
      </Provider>
    );
    unmount();

    render(
      <Provider store={buildStore()}>
        <AssistantUiRuntimeProvider>
          <AgentRunningStatus />
        </AssistantUiRuntimeProvider>
      </Provider>
    );
    expect(screen.getByTestId('agent-running-status-thinking')).toBeInTheDocument();
  });

  it('resets thinking timer when a new request starts', () => {
    vi.useFakeTimers();
    try {
      const now = 1000000;
      vi.setSystemTime(now);

      const store1 = buildStore({ liveRequestIdByThread: { [THREAD_ID]: 'req-1' } });
      const { rerender } = render(
        <Provider store={store1}>
          <AssistantUiRuntimeProvider>
            <AgentRunningStatus />
          </AssistantUiRuntimeProvider>
        </Provider>
      );
      expect(screen.getByTestId('agent-running-status-thinking')).toHaveTextContent('Thinking...');

      act(() => {
        vi.advanceTimersByTime(3000);
      });
      expect(screen.getByTestId('agent-running-status-thinking')).toHaveTextContent(
        /Thinking · 3\.0s/
      );

      const store2 = buildStore({ liveRequestIdByThread: { [THREAD_ID]: 'req-2' } });
      rerender(
        <Provider store={store2}>
          <AssistantUiRuntimeProvider>
            <AgentRunningStatus />
          </AssistantUiRuntimeProvider>
        </Provider>
      );
      expect(screen.getByTestId('agent-running-status-thinking')).toHaveTextContent('Thinking...');
    } finally {
      vi.useRealTimers();
    }
  });

  it('renders elapsed thinking label when time advances past 1 second', () => {
    vi.useFakeTimers();
    try {
      const now = 1000000;
      vi.setSystemTime(now);

      render(
        <Provider store={buildStore()}>
          <AssistantUiRuntimeProvider>
            <AgentRunningStatus />
          </AssistantUiRuntimeProvider>
        </Provider>
      );

      const thinkingEl = screen.getByTestId('agent-running-status-thinking');
      expect(thinkingEl).toHaveTextContent('Thinking...');

      act(() => {
        vi.advanceTimersByTime(3000);
      });

      expect(screen.getByTestId('agent-running-status-thinking')).toHaveTextContent(
        /Thinking · 3\.0s/
      );
    } finally {
      vi.useRealTimers();
    }
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
