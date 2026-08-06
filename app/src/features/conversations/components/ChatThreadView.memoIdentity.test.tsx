/**
 * Identity-stability regression tests for the transcript's live tool-timeline
 * and processing-transcript fallbacks (#5162).
 *
 * `selectedThreadToolTimeline` / `selectedThreadProcessing` used to fall back to
 * a freshly-allocated `[]` on every render. That gave the value a new identity
 * each pass, so the `backgroundProcesses` `useMemo` keyed on it was invalidated
 * every render and `selectBackgroundProcesses` re-ran for no reason — avoidable
 * churn on the chat's hot path, and the same identity mistake that fed the
 * render loop this change set fixes. Both now fall back to module-level
 * `EMPTY_*` constants, matching the convention the surrounding code already
 * used for the past-turn maps.
 *
 * The contract is "the memo is not invalidated by a re-render", so these tests
 * count `selectBackgroundProcesses` invocations across re-renders. The mock
 * delegates to the real selector, so behaviour is unchanged.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { render } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import chatRuntimeReducer from '../../../store/chatRuntimeSlice';
import themeReducer from '../../../store/themeSlice';
import threadReducer from '../../../store/threadSlice';
import { ChatThreadView } from './ChatThreadView';

const selectSpy = vi.hoisted(() => vi.fn());

vi.mock('./BackgroundProcessesPanel', async orig => {
  const actual = await orig<typeof import('./BackgroundProcessesPanel')>();
  return {
    ...actual,
    // Delegate to the real selector; we only need the call count.
    selectBackgroundProcesses: (...args: Parameters<typeof actual.selectBackgroundProcesses>) => {
      selectSpy();
      return actual.selectBackgroundProcesses(...args);
    },
  };
});

// Layout effects are meaningless in jsdom — same stub the sibling suites use.
vi.mock('../../../hooks/useStickToBottom', () => ({
  useStickToBottom: vi.fn(() => ({ containerRef: { current: null }, endRef: { current: null } })),
}));

const emptyThreadState = {
  threads: [],
  selectedThreadId: null,
  activeThreadIds: {},
  welcomeThreadId: null,
  messagesByThreadId: {},
  messages: [],
  isLoadingThreads: false,
  isLoadingMessages: false,
  messagesError: null,
};

function buildStore() {
  return configureStore({
    reducer: combineReducers({
      thread: threadReducer,
      chatRuntime: chatRuntimeReducer,
      theme: themeReducer,
    }),
    preloadedState: { thread: emptyThreadState } as never,
  });
}

describe('ChatThreadView empty-fallback identity (#5162)', () => {
  beforeEach(() => {
    selectSpy.mockClear();
  });

  it('does not re-derive background processes across re-renders with no thread', () => {
    const store = buildStore();
    const { rerender } = render(
      <Provider store={store}>
        <ChatThreadView threadId={null} />
      </Provider>
    );

    expect(selectSpy).toHaveBeenCalledTimes(1);

    // Nothing about the timeline changed, so the memo must hold. Pre-fix the
    // `?? []` fallback handed it a new array identity on each pass and this
    // climbed with every render.
    rerender(
      <Provider store={store}>
        <ChatThreadView threadId={null} />
      </Provider>
    );
    rerender(
      <Provider store={store}>
        <ChatThreadView threadId={null} />
      </Provider>
    );

    expect(selectSpy).toHaveBeenCalledTimes(1);
  });

  it('does not re-derive background processes for a thread with no timeline yet', () => {
    const store = buildStore();
    const props = { threadId: 't-1' };
    const { rerender } = render(
      <Provider store={store}>
        <ChatThreadView {...props} />
      </Provider>
    );

    expect(selectSpy).toHaveBeenCalledTimes(1);

    // The `toolTimelineByThread[threadId] ?? EMPTY_TOOL_TIMELINE` miss is the
    // common case for a fresh thread — it must be identity-stable too.
    rerender(
      <Provider store={store}>
        <ChatThreadView {...props} />
      </Provider>
    );

    expect(selectSpy).toHaveBeenCalledTimes(1);
  });
});
