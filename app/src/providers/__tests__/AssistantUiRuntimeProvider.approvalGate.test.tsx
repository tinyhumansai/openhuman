/**
 * The parked ApprovalGate, end to end through the real assistant-ui runtime.
 *
 * The migration to assistant-ui left `onApprovalRequest` writing
 * `chatRuntime.pendingApprovalByThread` while the only reader of that slice
 * (`ApprovalRequestCard`) lived inside `Conversations`' legacy panel, which
 * `/chat` never renders. The observable result was a shell call that spun
 * forever, no prompt, and a turn the core dropped as denied two minutes later.
 *
 * These tests pin both halves of the repair against the actual runtime rather
 * than the projection alone: the decision must reach the part, and pressing it
 * must reach `openhuman.approval_decide`.
 */
import {
  MessageByIndexProvider,
  PartByIndexProvider,
  useAui,
  useAuiState,
} from '@assistant-ui/react';
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../services/coreRpcClient';
import chatRuntimeReducer, {
  beginInferenceTurn,
  markInferenceTurnStreaming,
  setPendingApprovalForThread,
  toolCallReceived,
} from '../../store/chatRuntimeSlice';
import threadReducer from '../../store/threadSlice';
import type { ThreadMessage } from '../../types/thread';
import { AssistantUiRuntimeProvider } from '../AssistantUiRuntimeProvider';

vi.mock('../../services/api/threadApi', () => ({
  threadApi: {
    getDerivedTranscript: vi
      .fn()
      .mockResolvedValue({
        threadId: 't-gate',
        items: [],
        total: 0,
        hasMore: false,
        hasTranscript: false,
      }),
  },
}));

vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const THREAD_ID = 't-gate';
const REQUEST_ID = 'appr-7';

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

function buildStore() {
  return configureStore({
    reducer: combineReducers({ thread: threadReducer, chatRuntime: chatRuntimeReducer }),
    preloadedState: {
      thread: {
        threads: [],
        selectedThreadId: THREAD_ID,
        activeThreadIds: {},
        welcomeThreadId: null,
        messagesByThreadId: { [THREAD_ID]: [msg('q', 'user', 'list the repo')] },
        messages: [],
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
    } as never,
  });
}

type Store = ReturnType<typeof buildStore>;

/** A `shell` call parked on the gate, exactly as the socket layer records it. */
function parkShellCall(store: Store, toolName = 'shell') {
  store.dispatch(beginInferenceTurn({ threadId: THREAD_ID }));
  store.dispatch(markInferenceTurnStreaming({ threadId: THREAD_ID }));
  store.dispatch(
    toolCallReceived({ threadId: THREAD_ID, round: 1, toolName, toolCallId: 'call-1' })
  );
  store.dispatch(
    setPendingApprovalForThread({
      threadId: THREAD_ID,
      approval: {
        requestId: REQUEST_ID,
        toolName,
        message: 'Run a shell command?',
        command: 'ls -la',
      },
    })
  );
}

/**
 * Answers the gate through the very path a decision button uses: the part-scoped
 * `respondToToolApproval` assistant-ui hands every tool-call renderer.
 */
function PartProbe() {
  const aui = useAui();
  const part = useAuiState(state => state.part);
  return (
    <>
      <div data-testid="approval-id">
        {part.type === 'tool-call' ? (part.approval?.id ?? 'none') : 'none'}
      </div>
      <div data-testid="approval-tool">{part.type === 'tool-call' ? part.toolName : 'none'}</div>
      <div data-testid="approval-options">
        {part.type === 'tool-call'
          ? (part.approval?.options ?? []).map(option => option.id).join('|')
          : ''}
      </div>
      <div data-testid="part-status">{part.status?.type ?? 'none'}</div>
      <button
        type="button"
        data-testid="always-allow"
        onClick={() => aui.part.respondToToolApproval({ optionId: 'approve_always_for_tool' })}>
        always allow
      </button>
    </>
  );
}

/** Reports what the runtime believes about the gated call. */
function GateProbe() {
  const messages = useAuiState(state => state.thread.messages);
  const lastIndex = messages.length - 1;
  const last = messages[lastIndex];
  const partIndex =
    last?.role === 'assistant'
      ? last.content.findIndex(part => part.type === 'tool-call' && part.approval != null)
      : -1;
  return (
    <div>
      <div data-testid="message-status">{last?.status?.type ?? 'none'}</div>
      <div data-testid="message-reason">
        {last?.status && 'reason' in last.status ? String(last.status.reason) : 'none'}
      </div>
      {partIndex >= 0 ? (
        <MessageByIndexProvider index={lastIndex}>
          <PartByIndexProvider index={partIndex}>
            <PartProbe />
          </PartByIndexProvider>
        </MessageByIndexProvider>
      ) : (
        <div data-testid="approval-id">none</div>
      )}
    </div>
  );
}

function renderWith(store: Store) {
  return render(
    <Provider store={store}>
      <AssistantUiRuntimeProvider>
        <GateProbe />
      </AssistantUiRuntimeProvider>
    </Provider>
  );
}

beforeEach(() => {
  vi.mocked(callCoreRpc).mockReset();
  vi.mocked(callCoreRpc).mockResolvedValue(undefined as never);
});

afterEach(() => vi.clearAllMocks());

describe('parked approval on the assistant-ui surface', () => {
  it('hangs the decision off the tool part the gate is holding', async () => {
    const store = buildStore();
    renderWith(store);
    act(() => parkShellCall(store));

    await waitFor(() => expect(screen.getByTestId('approval-id')).toHaveTextContent(REQUEST_ID));
    // The prompt lands on the parked call itself, not on a synthesised row
    // beside it — the user reads the command from the card they answer.
    expect(screen.getByTestId('approval-tool')).toHaveTextContent('shell');
    expect(screen.getByTestId('approval-options')).toHaveTextContent(
      'approve_once|approve_always_for_tool|deny'
    );
  });

  it('marks the turn as awaiting the user rather than running', async () => {
    const store = buildStore();
    renderWith(store);
    act(() => parkShellCall(store));

    await waitFor(() =>
      expect(screen.getByTestId('message-status')).toHaveTextContent('requires-action')
    );
    expect(screen.getByTestId('message-reason')).toHaveTextContent('interrupt');
    // The part inherits it, which is the state a renderer keys the prompt off.
    expect(screen.getByTestId('part-status')).toHaveTextContent('requires-action');
  });

  it('routes a decision to openhuman.approval_decide and clears the gate', async () => {
    const store = buildStore();
    renderWith(store);
    act(() => parkShellCall(store));
    await waitFor(() => expect(screen.getByTestId('approval-id')).toHaveTextContent(REQUEST_ID));

    await act(async () => {
      screen.getByTestId('always-allow').click();
    });

    // Without an `onRespondToToolApproval` on the adapter this throws
    // "Runtime does not support tool approvals." and never reaches the RPC.
    await waitFor(() => expect(callCoreRpc).toHaveBeenCalledTimes(1));
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.approval_decide',
      params: { request_id: REQUEST_ID, decision: 'approve_always_for_tool' },
    });
    await waitFor(() =>
      expect(store.getState().chatRuntime.pendingApprovalByThread[THREAD_ID]).toBeUndefined()
    );
  });

  it('still prompts when the gate parks with no timeline row to hang it on', async () => {
    // The progress channel is bounded and drops frames under load, and the gate
    // can park before the `tool_call` frame lands at all. Dropping the prompt
    // in that case is the original bug in miniature.
    const store = buildStore();
    renderWith(store);
    act(() => {
      store.dispatch(beginInferenceTurn({ threadId: THREAD_ID }));
      store.dispatch(markInferenceTurnStreaming({ threadId: THREAD_ID }));
      store.dispatch(
        setPendingApprovalForThread({
          threadId: THREAD_ID,
          approval: { requestId: REQUEST_ID, toolName: 'write_file', message: 'Write?' },
        })
      );
    });

    await waitFor(() => expect(screen.getByTestId('approval-id')).toHaveTextContent(REQUEST_ID));
    expect(screen.getByTestId('approval-tool')).toHaveTextContent('write_file');
  });
});
