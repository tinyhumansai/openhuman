/**
 * The render half of the parked-approval repair.
 *
 * `AssistantUiChat` overrides assistant-ui's `ToolFallback` with
 * `ChatToolFallback`, and the component behind it destructured four fields and
 * dropped `status` and `approval` — so even once the runtime carried a decision,
 * nothing on screen offered it. The kit's own approval-capable fallback renders
 * on no user-facing surface (only the dev demo), which is why a grep for
 * approval support in `components/assistant-ui` looked healthy while the chat
 * had none.
 *
 * The controls are `ApprovalRequestCard`, the surface AGENTS.md designates for
 * this gate, so the core's action summary and the decision are one component.
 * A bespoke bar shipped first and had already lost the summary: it offered
 * "Always allow" for a `shell` call whose command the user could not read, and
 * `approve_always_for_tool` persists to the auto-approve allowlist.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { AssistantUiRuntimeProvider } from '../../../providers/AssistantUiRuntimeProvider';
import { callCoreRpc } from '../../../services/coreRpcClient';
import chatRuntimeReducer, {
  type PendingApproval,
  setPendingApprovalForThread,
} from '../../../store/chatRuntimeSlice';
import threadReducer from '../../../store/threadSlice';
import { ChatToolFallback } from './ChatToolParts';

vi.mock('../../../services/api/threadApi', () => ({
  threadApi: {
    getDerivedTranscript: vi
      .fn()
      .mockResolvedValue({
        threadId: 't-1',
        items: [],
        total: 0,
        hasMore: false,
        hasTranscript: false,
      }),
  },
}));

vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const THREAD_ID = 't-1';
const REQUEST_ID = 'appr-1';

/** The core's own explanation of what it is asking to do. */
const SUMMARY = 'Run shell — list the repository root';
const COMMAND = 'ls -la /Users/dev/secret-project';

/** The option set the projection puts on a parked call. */
const OPTIONS = [
  { id: 'approve_once', kind: 'allow-once' as const },
  { id: 'approve_always_for_tool', kind: 'allow-always' as const },
  { id: 'deny', kind: 'reject-once' as const },
];

const SHELL_REQUEST: PendingApproval = {
  requestId: REQUEST_ID,
  toolName: 'shell',
  message: SUMMARY,
  command: COMMAND,
};

function gatedPart(over: Record<string, unknown> = {}) {
  return {
    type: 'tool-call' as const,
    toolName: 'shell',
    toolCallId: 'call-1',
    // Deliberately argument-less: the gate can park before the `tool_call`
    // frame lands, and redacted args are the case the summary exists for.
    args: {} as never,
    argsText: '{}',
    result: undefined,
    status: { type: 'requires-action' as const, reason: 'interrupt' as const },
    approval: { id: REQUEST_ID, options: OPTIONS },
    addResult: () => {},
    resume: () => {},
    respondToApproval: () => {},
    ...over,
  };
}

function buildStore(approval?: PendingApproval) {
  const store = configureStore({
    reducer: combineReducers({ thread: threadReducer, chatRuntime: chatRuntimeReducer }),
    preloadedState: {
      thread: {
        threads: [],
        selectedThreadId: THREAD_ID,
        activeThreadIds: {},
        welcomeThreadId: null,
        messagesByThreadId: {},
        messages: [],
        isLoadingThreads: false,
        isLoadingMessages: false,
        messagesError: null,
      },
    } as never,
  });
  if (approval) store.dispatch(setPendingApprovalForThread({ threadId: THREAD_ID, approval }));
  return store;
}

/** Mounts the part under a runtime so `useAuiThreadId` resolves, as in the app. */
function renderInThread(node: React.ReactNode, approval?: PendingApproval) {
  return render(
    <Provider store={buildStore(approval)}>
      <AssistantUiRuntimeProvider>{node}</AssistantUiRuntimeProvider>
    </Provider>
  );
}

beforeEach(() => {
  vi.mocked(callCoreRpc).mockReset();
  vi.mocked(callCoreRpc).mockResolvedValue(undefined as never);
});

describe('ChatToolFallback — parked approval', () => {
  it('shows the core action summary and the exact command', () => {
    renderInThread(<ChatToolFallback {...gatedPart()} />, SHELL_REQUEST);

    expect(screen.getByText(SUMMARY)).toBeInTheDocument();
    expect(screen.getByText(COMMAND)).toBeInTheDocument();
  });

  it('keeps every decision control inside the card that carries the summary', () => {
    // The consent surface must show what is being consented to. A control that
    // can render without the summary beside it is the defect this pins:
    // `approve_always_for_tool` writes the auto-approve allowlist, so deciding
    // blind is a durable mistake, not a recoverable one.
    renderInThread(<ChatToolFallback {...gatedPart()} />, SHELL_REQUEST);

    const card = screen.getByRole('alertdialog');
    expect(within(card).getByText(SUMMARY)).toBeInTheDocument();
    expect(within(card).getByText(COMMAND)).toBeInTheDocument();
    for (const name of ['Approve', 'Always allow', 'Deny']) {
      expect(within(card).getByRole('button', { name })).toBeInTheDocument();
    }
    // And nowhere else on the surface.
    expect(screen.getAllByRole('button', { name: 'Approve' })).toHaveLength(1);
    expect(screen.getAllByRole('button', { name: 'Always allow' })).toHaveLength(1);
  });

  it('says the call is awaiting input, not merely running', () => {
    // The `awaiting input` label existed but was unreachable: its only trigger
    // was a `status` the adapter forwards for `error` / `cancelled` alone.
    renderInThread(<ChatToolFallback {...gatedPart()} />, SHELL_REQUEST);

    expect(screen.getByText('awaiting input')).toBeInTheDocument();
    expect(screen.queryByText('running')).not.toBeInTheDocument();
  });

  it('routes a decision to openhuman.approval_decide', async () => {
    const store = buildStore(SHELL_REQUEST);
    render(
      <Provider store={store}>
        <AssistantUiRuntimeProvider>
          <ChatToolFallback {...gatedPart()} />
        </AssistantUiRuntimeProvider>
      </Provider>
    );

    await userEvent.click(screen.getByRole('button', { name: 'Always allow' }));

    await waitFor(() => expect(callCoreRpc).toHaveBeenCalledTimes(1));
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.approval_decide',
      params: { request_id: REQUEST_ID, decision: 'approve_always_for_tool' },
    });
    await waitFor(() =>
      expect(store.getState().chatRuntime.pendingApprovalByThread[THREAD_ID]).toBeUndefined()
    );
  });

  it('re-offers the decision when the decide fails', async () => {
    // Reusing the card buys this for free: a bespoke bar could not see the
    // rejection, because the runtime swallows it.
    vi.mocked(callCoreRpc).mockRejectedValue(new Error('core unreachable'));
    renderInThread(<ChatToolFallback {...gatedPart()} />, SHELL_REQUEST);

    await userEvent.click(screen.getByRole('button', { name: 'Approve' }));

    await waitFor(() =>
      expect(screen.getByText(/Could not record your decision/)).toBeInTheDocument()
    );
    expect(screen.getByRole('button', { name: 'Approve' })).toBeEnabled();
  });

  it('leaves an ordinary running tool alone', () => {
    // Every unsettled tool part in a `requires-action` message inherits that
    // status, so the prompt must key off the store's request, not the status.
    renderInThread(
      <ChatToolFallback
        {...gatedPart({ toolName: 'web_search', approval: undefined, toolCallId: 'call-2' })}
      />,
      SHELL_REQUEST
    );

    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
    expect(screen.queryByText('awaiting input')).not.toBeInTheDocument();
  });

  it('gives composio_connect the OAuth affordance rather than approve/deny', () => {
    // `composio_connect` parks on the same gate, but approving it without
    // connecting resumes the agent against a toolkit with no credentials.
    renderInThread(
      <ChatToolFallback
        {...gatedPart({ toolName: 'composio_connect', args: { toolkit: 'googledrive' } })}
      />,
      {
        requestId: REQUEST_ID,
        toolName: 'composio_connect',
        message: 'Connect Google Drive?',
        toolkit: 'googledrive',
      }
    );

    expect(screen.getByTestId('assistant-ui-integration-connect')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /connect/i })).toBeInTheDocument();
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
  });

  it('falls back to the ordinary card when the part is not the parked request', () => {
    // A stale part from an earlier turn must not adopt the live request — it
    // would render someone else's summary above a live decision.
    renderInThread(
      <ChatToolFallback {...gatedPart({ approval: { id: 'appr-stale', options: OPTIONS } })} />,
      SHELL_REQUEST
    );

    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
    expect(screen.queryByText(SUMMARY)).not.toBeInTheDocument();
  });
});
