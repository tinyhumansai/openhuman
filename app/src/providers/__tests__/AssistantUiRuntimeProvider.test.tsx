/**
 * End-to-end proof of the adapter seam: a component using assistant-ui's own
 * hooks, mounted under the runtime, must see exactly what Redux holds — and
 * must keep seeing it as Redux changes.
 *
 * This is the test that makes the adoption meaningful. The unit tests around
 * `assistantUiMessages` prove the projection; this proves the runtime consumes
 * it, so the runtime's view of the conversation and the store cannot drift.
 */
import { useAui, useAuiState } from '@assistant-ui/react';
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { threadApi } from '../../services/api/threadApi';
import chatRuntimeReducer, {
  beginInferenceTurn,
  markInferenceTurnStreaming,
  streamDeltaReceived,
} from '../../store/chatRuntimeSlice';
import threadReducer from '../../store/threadSlice';
import type { ThreadMessage } from '../../types/thread';
import { AssistantUiRuntimeProvider } from '../AssistantUiRuntimeProvider';
import { __resetChatSurfaces, registerChatSurface } from '../chatSurfaceHandlers';

vi.mock('../../services/api/threadApi', () => ({
  threadApi: {
    getDerivedTranscript: vi
      .fn()
      .mockResolvedValue({
        threadId: 't-aui',
        items: [],
        total: 0,
        hasMore: false,
        hasTranscript: false,
      }),
  },
}));

const THREAD_ID = 't-aui';

function msg(id: string, sender: 'user' | 'agent', content: string): ThreadMessage {
  return {
    id,
    sender,
    type: 'text',
    content,
    extraMetadata: {},
    createdAt: '2026-01-01T00:00:00.000Z',
  };
}

function buildStore(messages: ThreadMessage[]) {
  return configureStore({
    reducer: combineReducers({ thread: threadReducer, chatRuntime: chatRuntimeReducer }),
    preloadedState: {
      thread: {
        threads: [],
        selectedThreadId: THREAD_ID,
        activeThreadIds: {},
        welcomeThreadId: null,
        messagesByThreadId: { [THREAD_ID]: messages },
        messages: [],
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
    } as never,
  });
}

/** Renders what assistant-ui's runtime believes the thread contains. */
function RuntimeProbe() {
  const thread = useAuiState(({ thread: t }) => t);
  return (
    <div>
      <div data-testid="count">{thread.messages.length}</div>
      <div data-testid="running">{String(thread.isRunning)}</div>
      <div data-testid="text">
        {thread.messages
          .map(m => m.content.map(p => (p.type === 'text' ? p.text : '')).join(''))
          .join('|')}
      </div>
      <div data-testid="tools">
        {thread.messages
          .flatMap(message =>
            message.role === 'assistant'
              ? message.content.flatMap(part => (part.type === 'tool-call' ? [part.toolName] : []))
              : []
          )
          .join('|')}
      </div>
    </div>
  );
}

function renderWith(store: ReturnType<typeof buildStore>) {
  return render(
    <Provider store={store}>
      <AssistantUiRuntimeProvider>
        <RuntimeProbe />
      </AssistantUiRuntimeProvider>
    </Provider>
  );
}

afterEach(() => __resetChatSurfaces());

describe('AssistantUiRuntimeProvider', () => {
  it('exposes the Redux transcript through assistant-ui hooks', () => {
    renderWith(buildStore([msg('a', 'user', 'question'), msg('b', 'agent', 'answer')]));
    expect(screen.getByTestId('count')).toHaveTextContent('2');
    expect(screen.getByTestId('text')).toHaveTextContent('question|answer');
  });

  it('renders an empty thread without a live tail', () => {
    renderWith(buildStore([]));
    expect(screen.getByTestId('count')).toHaveTextContent('0');
  });

  it('surfaces the live stream as a running tail message', async () => {
    const store = buildStore([msg('a', 'user', 'question')]);
    renderWith(store);
    expect(screen.getByTestId('count')).toHaveTextContent('1');

    act(() => {
      store.dispatch(beginInferenceTurn({ threadId: THREAD_ID }));
      store.dispatch(markInferenceTurnStreaming({ threadId: THREAD_ID }));
      store.dispatch(
        streamDeltaReceived({
          threadId: THREAD_ID,
          requestId: 'req-1',
          round: 0,
          delta: 'partial answer',
          channel: 'content',
        })
      );
    });

    await waitFor(() => expect(screen.getByTestId('count')).toHaveTextContent('2'));
    expect(screen.getByTestId('text')).toHaveTextContent('question|partial answer');
  });

  it('reads settled reasoning and tools directly from the core transcript RPC', async () => {
    vi.mocked(threadApi.getDerivedTranscript).mockResolvedValueOnce({
      threadId: THREAD_ID,
      // RPC pages are newest-first: reverse traversal sees the boundary first.
      items: [
        {
          kind: 'toolCall',
          callId: 'call-web',
          name: 'web_fetch',
          args: { url: 'https://example.com' },
          result: 'Example Domain',
          status: 'success',
        },
        { kind: 'reasoning', text: 'I should fetch the source.' },
        { kind: 'turnBoundary', requestId: 'req-core' },
      ],
      total: 3,
      hasMore: false,
      hasTranscript: true,
    });
    const answer = msg('answer', 'agent', 'done');
    answer.extraMetadata = { requestId: 'req-core' };

    renderWith(buildStore([msg('question', 'user', 'fetch it'), answer]));

    await waitFor(() => expect(screen.getByTestId('tools')).toHaveTextContent('web_fetch'));
    expect(threadApi.getDerivedTranscript).toHaveBeenCalledWith(THREAD_ID, { limit: 500 });
  });

  it('forwards onNew to the surface that owns the thread', async () => {
    const send = vi.fn(async () => {});
    registerChatSurface(THREAD_ID, { send });

    function Sender() {
      const aui = useAui();
      return (
        <button
          type="button"
          data-testid="send"
          onClick={() =>
            void aui.thread.append({ role: 'user', content: [{ type: 'text', text: 'hi' }] })
          }>
          send
        </button>
      );
    }

    render(
      <Provider store={buildStore([])}>
        <AssistantUiRuntimeProvider>
          <Sender />
        </AssistantUiRuntimeProvider>
      </Provider>
    );

    await act(async () => {
      screen.getByTestId('send').click();
    });

    expect(send).toHaveBeenCalledWith('hi');
  });
});
