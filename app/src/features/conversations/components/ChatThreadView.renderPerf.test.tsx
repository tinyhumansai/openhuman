/**
 * Render-cost regression benchmark for the chat transcript.
 *
 * The defect this pins (verified before any fix existed):
 *
 * `ChatThreadView` calls `unwrapToolCallEnvelope(msg.content)` inside the
 * `timelineMessages.map()` in the RENDER BODY — unmemoized, once per agent
 * message, on every single render. `unwrapToolCallEnvelope` does a
 * `JSON.parse` in a `try/catch`, and for ordinary assistant prose (the common
 * case, by far) that parse THROWS and is caught. Nothing below the transcript
 * is memoized either: `React.memo` appears zero times in `ChatThreadView.tsx`,
 * `AgentMessageBubble.tsx` and `ToolTimelineBlock.tsx`.
 *
 * `ChatThreadView` re-renders on every streamed token (the live preview reads
 * `state.chatRuntime.streamingAssistantByThread`). So each token re-parses
 * every agent message already settled in the transcript: O(N) throwaway work
 * per token, N = transcript length.
 *
 * The contract asserted here is the one that must hold AFTER the fix:
 *
 *   1. Appending a token to the live tail must not re-unwrap the ~39 settled
 *      messages above it. Expressed as a small constant bound, not an exact
 *      count, so it does not rot on unrelated transcript changes.
 *   2. A settled (non-tail) message's bubble must not re-render when only the
 *      tail changes.
 *
 * Both are stated as bounds on work per tail update, and both fail on the
 * pre-fix code. Store/provider harness is deliberately the same one
 * `ChatThreadView.test.tsx` and `ChatThreadView.memoIdentity.test.tsx`
 * already use — no new fixtures.
 *
 * This file measures work, not markup: it must not be "fixed" by changing
 * what the transcript renders. Rendered output is unchanged by design.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, render } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import chatRuntimeReducer, { streamDeltaReceived } from '../../../store/chatRuntimeSlice';
import themeReducer from '../../../store/themeSlice';
import threadReducer from '../../../store/threadSlice';
import type { Thread, ThreadMessage } from '../../../types/thread';
import { ChatThreadView } from './ChatThreadView';

/** Counts every `unwrapToolCallEnvelope` call, wherever in the tree it happens. */
const unwrapSpy = vi.hoisted(() => vi.fn());
/** Records the `content` of every agent message body React actually renders. */
const bubbleRenderSpy = vi.hoisted(() => vi.fn<(content: string) => void>());

// Spy at the module boundary rather than on `JSON.parse` directly: the count
// then stays correct if the fix MOVES the unwrap (e.g. down into a memoized
// per-message component) instead of caching it in place. Delegates to the real
// implementation, so behaviour is untouched.
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

// Transparent pass-through, deliberately NOT wrapped in `memo`: the contract is
// that the PARENT stops re-rendering settled message rows, so the instrument
// must not supply memoization of its own and mask that.
vi.mock('./AgentMessageBubble', async orig => {
  const actual = await orig<typeof import('./AgentMessageBubble')>();
  return {
    ...actual,
    // Both agent-body renderers are instrumented: `themeSlice` defaults
    // `agentMessageViewMode` to `'text'`, so `AgentMessageText` is the one the
    // default transcript actually mounts — but a host that flips the mode to
    // `'bubbles'` must hold the same contract.
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

// Layout effects are meaningless in jsdom — same stub the sibling suites use.
vi.mock('../../../hooks/useStickToBottom', () => ({
  useStickToBottom: vi.fn(() => ({ containerRef: { current: null }, endRef: { current: null } })),
}));

const THREAD_ID = 't-perf';
const MESSAGE_COUNT = 40;

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

function makeThread(): Thread {
  return {
    id: THREAD_ID,
    title: 'Perf thread',
    chatId: null,
    isActive: true,
    messageCount: MESSAGE_COUNT,
    lastMessageAt: '2026-01-01T00:00:00.000Z',
    createdAt: '2026-01-01T00:00:00.000Z',
    labels: ['general'],
  };
}

/**
 * ~40 messages alternating user/agent. Every agent message is ordinary PROSE —
 * that is the common case, and the one where `JSON.parse` throws and is caught
 * on each call. One agent message carries a real tool-call envelope so the
 * benchmark also covers the branch that legitimately parses.
 */
function buildTranscript(tailContent: string): ThreadMessage[] {
  const messages: ThreadMessage[] = [];
  for (let i = 0; i < MESSAGE_COUNT - 1; i += 1) {
    const isAgent = i % 2 === 1;
    const envelope = i === 21;
    messages.push({
      id: `m-${i}`,
      sender: isAgent ? 'agent' : 'user',
      type: 'text',
      content: envelope
        ? JSON.stringify({
            content: 'Pulling that up now.',
            tool_calls: [{ id: 'call_1', name: 'memory_search', arguments: '{}' }],
          })
        : isAgent
          ? `Settled agent prose MARK${i}_ nothing JSON about it at all`
          : `User question ${i}?`,
      extraMetadata: {},
      createdAt: new Date(Date.UTC(2026, 0, 1, 0, i)).toISOString(),
    });
  }
  // The tail: the agent message a socket is streaming into.
  messages.push({
    id: 'm-tail',
    sender: 'agent',
    type: 'text',
    content: tailContent,
    extraMetadata: {},
    createdAt: new Date(Date.UTC(2026, 0, 1, 1, 0)).toISOString(),
  });
  return messages;
}

function buildStore(messages: ThreadMessage[]) {
  return configureStore({
    reducer: combineReducers({
      thread: threadReducer,
      chatRuntime: chatRuntimeReducer,
      theme: themeReducer,
    }),
    preloadedState: {
      thread: {
        ...emptyThreadState,
        threads: [makeThread()],
        selectedThreadId: THREAD_ID,
        messagesByThreadId: { [THREAD_ID]: messages },
      },
    } as never,
  });
}

/**
 * Work permitted for a single appended token. Generous on purpose: the fix
 * only has to stop the transcript-wide sweep, so anything on the order of the
 * live tail (and its immediate neighbours) is fine. It is the ~40-per-token
 * sweep that must go. A bound, not an exact number, so incidental transcript
 * changes do not rot the test.
 */
const MAX_WORK_PER_TAIL_TOKEN = 4;

describe('ChatThreadView transcript render cost under streaming', () => {
  beforeEach(() => {
    unwrapSpy.mockClear();
    bubbleRenderSpy.mockClear();
  });

  it('does not re-unwrap the settled transcript on every streamed token', () => {
    const store = buildStore(buildTranscript('Starting the answer'));
    render(
      <Provider store={store}>
        <ChatThreadView threadId={THREAD_ID} />
      </Provider>
    );

    // Baseline mount is allowed to unwrap every agent message once.
    expect(unwrapSpy.mock.calls.length).toBeGreaterThan(0);
    unwrapSpy.mockClear();

    // Stream, as the socket does: `streamDeltaReceived` per token. Nothing in
    // `state.thread` changes — the settled transcript is byte-for-byte the same
    // array — so re-unwrapping any of it is pure waste.
    const TOKENS = 5;
    for (let i = 0; i < TOKENS; i += 1) {
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

    expect(unwrapSpy.mock.calls.length).toBeLessThanOrEqual(MAX_WORK_PER_TAIL_TOKEN * TOKENS);
  });

  it('does not re-unwrap settled messages when only the tail message grows', () => {
    // The other shape of the same event: the tail ThreadMessage itself grows as
    // the answer lands. Everything above it is unchanged and must not be
    // touched again.
    const { rerender } = (() => {
      const store = buildStore(buildTranscript('Tok'));
      const utils = render(
        <Provider store={store}>
          <ChatThreadView threadId={THREAD_ID} />
        </Provider>
      );
      return utils;
    })();

    unwrapSpy.mockClear();

    const GROWTHS = 5;
    let tail = 'Tok';
    for (let i = 0; i < GROWTHS; i += 1) {
      tail = `${tail} tok${i}`;
      const nextStore = buildStore(buildTranscript(tail));
      rerender(
        <Provider store={nextStore}>
          <ChatThreadView threadId={THREAD_ID} />
        </Provider>
      );
    }

    expect(unwrapSpy.mock.calls.length).toBeLessThanOrEqual(MAX_WORK_PER_TAIL_TOKEN * GROWTHS);
  });

  it('does not re-render a settled message bubble when only the tail changes', () => {
    const store = buildStore(buildTranscript('Starting the answer'));
    render(
      <Provider store={store}>
        <ChatThreadView threadId={THREAD_ID} />
      </Provider>
    );

    // Pick one stable, non-tail agent message and watch only that bubble.
    // Matched by its unique marker rather than by whole-string equality:
    // `splitAgentMessageIntoBubbles` may hand the bubble a segment rather than
    // the full message, and that split is not what is under test here.
    const STABLE_MARKER = 'MARK1_';
    const rendersOfStable = () =>
      bubbleRenderSpy.mock.calls.filter(([c]) => c.includes(STABLE_MARKER)).length;

    // Sanity check on the instrument itself: if the mount never rendered this
    // bubble, the assertion below would pass vacuously and measure nothing.
    expect(rendersOfStable()).toBe(1);
    bubbleRenderSpy.mockClear();

    for (let i = 0; i < 5; i += 1) {
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

    // A message nobody touched must not be re-rendered by a token landing on
    // the live tail.
    expect(rendersOfStable()).toBe(0);
  });
});
