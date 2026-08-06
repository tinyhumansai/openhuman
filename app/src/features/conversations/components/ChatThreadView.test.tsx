/**
 * Focused behavior tests for the extracted transcript scroll body. Mirrors
 * the store-mounting conventions used by
 * `src/pages/__tests__/Conversations.render.test.tsx` (the home chat's own
 * smoke suite), but scoped to `ChatThreadView` in isolation so a second host
 * (e.g. the Workflow Copilot) can be confident the component works when
 * driven purely by its `threadId` prop rather than the global
 * `state.thread.selectedThreadId`.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import chatRuntimeReducer from '../../../store/chatRuntimeSlice';
import themeReducer from '../../../store/themeSlice';
import threadReducer from '../../../store/threadSlice';
import type { Thread, ThreadMessage } from '../../../types/thread';
import { ChatThreadView } from './ChatThreadView';

// useStickToBottom returns refs; mock it so layout-effects don't fire in
// jsdom (same stub the home-chat render suite uses).
vi.mock('../../../hooks/useStickToBottom', () => ({
  useStickToBottom: vi.fn(() => ({ containerRef: { current: null }, endRef: { current: null } })),
}));

function makeThread(overrides: Partial<Thread> = {}): Thread {
  return {
    id: 't-1',
    title: 'Test thread',
    chatId: null,
    isActive: false,
    messageCount: 0,
    lastMessageAt: '2026-01-01T00:00:00.000Z',
    createdAt: '2026-01-01T00:00:00.000Z',
    labels: ['general'],
    ...overrides,
  };
}

function buildStore(preload: Record<string, unknown> = {}) {
  return configureStore({
    reducer: combineReducers({
      thread: threadReducer,
      chatRuntime: chatRuntimeReducer,
      theme: themeReducer,
    }),
    preloadedState: preload as never,
  });
}

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

function renderThreadView(
  props: Partial<ComponentProps<typeof ChatThreadView>> & { threadId: string | null },
  preload: Record<string, unknown> = {}
) {
  const store = buildStore(preload);
  render(
    <Provider store={store}>
      <ChatThreadView {...props} />
    </Provider>
  );
  return store;
}

describe('ChatThreadView', () => {
  it('renders the chat-messages-scroll container', () => {
    renderThreadView({ threadId: null }, { thread: emptyThreadState });

    expect(screen.getByTestId('chat-messages-scroll')).toBeInTheDocument();
  });

  it('shows emptyContent when the thread has no messages, footer content, or live activity', () => {
    const thread = makeThread({ id: 't-empty' });
    renderThreadView(
      { threadId: thread.id, emptyContent: <p>Nothing here yet</p> },
      {
        thread: {
          ...emptyThreadState,
          threads: [thread],
          selectedThreadId: thread.id,
          messagesByThreadId: { [thread.id]: [] },
        },
      }
    );

    expect(screen.getByText('Nothing here yet')).toBeInTheDocument();
    expect(screen.queryByTestId('chat-message-list')).not.toBeInTheDocument();
  });

  it('renders a user and an agent message for the given threadId', () => {
    const thread = makeThread({ id: 't-msgs' });
    const messages: ThreadMessage[] = [
      {
        id: 'm-user',
        sender: 'user',
        type: 'text',
        content: 'Hello there',
        extraMetadata: {},
        createdAt: '2026-01-01T00:00:00.000Z',
      },
      {
        id: 'm-agent',
        sender: 'agent',
        type: 'text',
        content: 'General Kenobi',
        extraMetadata: {},
        createdAt: '2026-01-01T00:01:00.000Z',
      },
    ];

    renderThreadView(
      { threadId: thread.id },
      {
        thread: {
          ...emptyThreadState,
          threads: [thread],
          selectedThreadId: thread.id,
          messagesByThreadId: { [thread.id]: messages },
        },
      }
    );

    expect(screen.getByTestId('chat-message-list')).toBeInTheDocument();
    expect(screen.getByText('Hello there')).toBeInTheDocument();
    expect(screen.getByText('General Kenobi')).toBeInTheDocument();
  });

  it('reads messages by the threadId prop, not a global selected thread', () => {
    // Two threads hydrated in the store; only the prop-driven thread's
    // messages should render — proving the component doesn't fall back to
    // `state.thread.selectedThreadId`.
    const threadA = makeThread({ id: 't-a' });
    const threadB = makeThread({ id: 't-b' });
    const messagesA: ThreadMessage[] = [
      {
        id: 'a-1',
        sender: 'user',
        type: 'text',
        content: 'Message in thread A',
        extraMetadata: {},
        createdAt: '2026-01-01T00:00:00.000Z',
      },
    ];
    const messagesB: ThreadMessage[] = [
      {
        id: 'b-1',
        sender: 'user',
        type: 'text',
        content: 'Message in thread B',
        extraMetadata: {},
        createdAt: '2026-01-01T00:00:00.000Z',
      },
    ];

    renderThreadView(
      { threadId: threadB.id },
      {
        thread: {
          ...emptyThreadState,
          threads: [threadA, threadB],
          // Deliberately select A globally — the component must still
          // render B's messages because it reads `threadId` prop, not
          // `selectedThreadId`.
          selectedThreadId: threadA.id,
          messagesByThreadId: { [threadA.id]: messagesA, [threadB.id]: messagesB },
        },
      }
    );

    expect(screen.getByText('Message in thread B')).toBeInTheDocument();
    expect(screen.queryByText('Message in thread A')).not.toBeInTheDocument();
  });

  it('B25: unwraps a raw tool-call envelope agent message to clean text, never raw JSON', () => {
    // A `workflow_builder` turn that both talks AND calls a tool can land in
    // the transcript as the provider wire-format `{ content, tool_calls }`
    // envelope. The shared renderer (used by both the home chat and the
    // workflow copilot) must show only the human text — never the raw JSON.
    const thread = makeThread({ id: 't-envelope' });
    const messages: ThreadMessage[] = [
      {
        id: 'm-user',
        sender: 'user',
        type: 'text',
        content: 'build me a Slack digest',
        extraMetadata: {},
        createdAt: '2026-01-01T00:00:00.000Z',
      },
      {
        id: 'm-agent',
        sender: 'agent',
        type: 'text',
        content: JSON.stringify({
          content: "Here's the workflow I propose.",
          tool_calls: [{ id: 'call_1', name: 'propose_workflow', arguments: '{"nodes":[]}' }],
        }),
        extraMetadata: {},
        createdAt: '2026-01-01T00:01:00.000Z',
      },
    ];

    renderThreadView(
      { threadId: thread.id },
      {
        thread: {
          ...emptyThreadState,
          threads: [thread],
          selectedThreadId: thread.id,
          messagesByThreadId: { [thread.id]: messages },
        },
      }
    );

    const list = screen.getByTestId('chat-message-list');
    expect(list).toHaveTextContent("Here's the workflow I propose.");
    // The raw envelope must never reach the DOM as text.
    expect(list).not.toHaveTextContent('tool_calls');
    expect(list).not.toHaveTextContent('"nodes":[]');
  });
});
