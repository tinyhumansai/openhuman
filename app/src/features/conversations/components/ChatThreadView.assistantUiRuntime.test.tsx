/**
 * Guards the assistant-ui adoption against the one regression that would make
 * it not worth having.
 *
 * `ChatThreadView.renderPerf.test.tsx` pins the transcript's render cost, but
 * it mounts the transcript under a bare Redux `Provider` — the assistant-ui
 * runtime is not in that tree, so it cannot see a cost the runtime introduces.
 * `App.tsx` mounts the runtime ABOVE every chat surface, and a runtime that
 * re-rendered or re-converted the transcript per streamed token would undo the
 * memoized-`TranscriptRow` optimisation without failing a single existing test.
 *
 * So this file re-states the same property with the runtime in place. The
 * bound and the instrument are deliberately the same ones the sibling
 * benchmark uses; only the tree differs. Measured at adoption time, the counts
 * are identical with and without the runtime mounted (0 unwraps per streamed
 * token, 0 settled-bubble re-renders).
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, render } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { AssistantUiRuntimeProvider } from '../../../providers/AssistantUiRuntimeProvider';
import chatRuntimeReducer, { streamDeltaReceived } from '../../../store/chatRuntimeSlice';
import themeReducer from '../../../store/themeSlice';
import threadReducer from '../../../store/threadSlice';
import type { ThreadMessage } from '../../../types/thread';
import { ChatThreadView } from './ChatThreadView';

const unwrapSpy = vi.hoisted(() => vi.fn());
const bubbleRenderSpy = vi.hoisted(() => vi.fn<(content: string) => void>());

vi.mock('../../../lib/chat/toolCallEnvelope', async orig => {
  const actual = await orig<typeof import('../../../lib/chat/toolCallEnvelope')>();
  return {
    ...actual,
    unwrapToolCallEnvelope: (raw: string) => {
      unwrapSpy(raw);
      return actual.unwrapToolCallEnvelope(raw);
    },
  };
});

// Pass-through, deliberately unmemoized: the contract is that the PARENT stops
// re-rendering settled rows, so the instrument must not supply memoization.
vi.mock('./AgentMessageBubble', async orig => {
  const actual = await orig<typeof import('./AgentMessageBubble')>();
  return {
    ...actual,
    AgentMessageText: (props: Parameters<typeof actual.AgentMessageText>[0]) => {
      bubbleRenderSpy(props.content);
      return actual.AgentMessageText(props);
    },
    AgentMessageBubble: (props: Parameters<typeof actual.AgentMessageBubble>[0]) => {
      bubbleRenderSpy(props.content);
      return actual.AgentMessageBubble(props);
    },
  };
});

vi.mock('../../../hooks/useStickToBottom', () => ({
  useStickToBottom: vi.fn(() => ({ containerRef: { current: null }, endRef: { current: null } })),
}));

const THREAD_ID = 't-aui-perf';
const MESSAGE_COUNT = 40;
const MAX_WORK_PER_TAIL_TOKEN = 4;

function buildTranscript(): ThreadMessage[] {
  const messages: ThreadMessage[] = [];
  for (let i = 0; i < MESSAGE_COUNT - 1; i += 1) {
    const isAgent = i % 2 === 1;
    messages.push({
      id: `m-${i}`,
      sender: isAgent ? 'agent' : 'user',
      type: 'text',
      content: isAgent ? `Settled agent prose MARK${i}_ not JSON` : `User question ${i}?`,
      extraMetadata: {},
      createdAt: new Date(Date.UTC(2026, 0, 1, 0, i)).toISOString(),
    });
  }
  messages.push({
    id: 'm-tail',
    sender: 'agent',
    type: 'text',
    content: 'Starting the answer',
    extraMetadata: {},
    createdAt: new Date(Date.UTC(2026, 0, 1, 1, 0)).toISOString(),
  });
  return messages;
}

function buildStore() {
  return configureStore({
    reducer: combineReducers({
      thread: threadReducer,
      chatRuntime: chatRuntimeReducer,
      theme: themeReducer,
    }),
    preloadedState: {
      thread: {
        threads: [
          {
            id: THREAD_ID,
            title: 'Perf thread',
            chatId: null,
            isActive: true,
            messageCount: MESSAGE_COUNT,
            lastMessageAt: '2026-01-01T00:00:00.000Z',
            createdAt: '2026-01-01T00:00:00.000Z',
            labels: ['general'],
          },
        ],
        selectedThreadId: THREAD_ID,
        activeThreadIds: {},
        welcomeThreadId: null,
        messagesByThreadId: { [THREAD_ID]: buildTranscript() },
        messages: [],
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
    } as never,
  });
}

function renderUnderRuntime(store: ReturnType<typeof buildStore>) {
  return render(
    <Provider store={store}>
      <AssistantUiRuntimeProvider>
        <ChatThreadView threadId={THREAD_ID} />
      </AssistantUiRuntimeProvider>
    </Provider>
  );
}

function streamTokens(store: ReturnType<typeof buildStore>, count: number) {
  for (let i = 0; i < count; i += 1) {
    act(() => {
      store.dispatch(
        streamDeltaReceived({
          threadId: THREAD_ID,
          requestId: 'req-1',
          round: 0,
          delta: ` tok${i}`,
          channel: 'content',
        })
      );
    });
  }
}

describe('transcript render cost with the assistant-ui runtime mounted', () => {
  beforeEach(() => {
    unwrapSpy.mockClear();
    bubbleRenderSpy.mockClear();
  });

  it('does not re-unwrap the settled transcript on every streamed token', () => {
    const store = buildStore();
    renderUnderRuntime(store);
    expect(unwrapSpy.mock.calls.length).toBeGreaterThan(0);
    unwrapSpy.mockClear();

    const TOKENS = 5;
    streamTokens(store, TOKENS);

    expect(unwrapSpy.mock.calls.length).toBeLessThanOrEqual(MAX_WORK_PER_TAIL_TOKEN * TOKENS);
  });

  it('does not re-render a settled message bubble when only the tail changes', () => {
    const store = buildStore();
    renderUnderRuntime(store);

    const STABLE_MARKER = 'MARK1_';
    const rendersOfStable = () =>
      bubbleRenderSpy.mock.calls.filter(([c]) => c.includes(STABLE_MARKER)).length;

    // Guards the instrument: a vacuous pass would measure nothing.
    expect(rendersOfStable()).toBe(1);
    bubbleRenderSpy.mockClear();

    streamTokens(store, 5);

    expect(rendersOfStable()).toBe(0);
  });
});
