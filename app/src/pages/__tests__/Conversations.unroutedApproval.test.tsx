/**
 * The background-approval banner, proven from the LIVE `/chat` route.
 *
 * The bug being fixed (openhuman#6406) is entirely one of reachability: the
 * approval gate parks a background trigger's tool call, the row is persisted
 * and listable, `approval_decide` works — and no component ever asks for it,
 * so the park TTL-denies with the user never shown anything. A test that
 * mounted the card directly would prove nothing about that, which is the whole
 * point of the defect. So this renders `Conversations` at `/chat/:threadId?`,
 * the real route element, and asserts the banner arrives on its own.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Provider } from 'react-redux';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { SidebarSlotOutlet, SidebarSlotProvider } from '../../components/layout/shell/SidebarSlot';
import { resetFlowPendingApprovalsStoreForTests } from '../../hooks/flowPendingApprovalsStore';
import {
  decideApproval,
  fetchPendingApprovals,
  type PendingApproval,
} from '../../services/api/approvalApi';
import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import layoutReducer from '../../store/layoutSlice';
import socketReducer from '../../store/socketSlice';
import themeReducer from '../../store/themeSlice';
import threadReducer from '../../store/threadSlice';

vi.mock('../../services/api/approvalApi', async importOriginal => {
  const actual = await importOriginal<typeof import('../../services/api/approvalApi')>();
  return { ...actual, fetchPendingApprovals: vi.fn(), decideApproval: vi.fn() };
});
vi.mock('../../services/chatService', async importOriginal => {
  // Spread the original rather than listing exports: this module has grown
  // several (`useRustChat`, …) and a hand-written list silently breaks the
  // whole render the next time one is added.
  const actual = await importOriginal<typeof import('../../services/chatService')>();
  return {
    ...actual,
    chatCancel: vi.fn().mockResolvedValue({ accepted: true, turnCancelled: true }),
    chatClearQueue: vi.fn().mockResolvedValue(0),
    chatSend: vi.fn().mockResolvedValue(undefined),
  };
});
vi.mock('../../services/api/threadApi', () => ({
  threadApi: {
    createNewThread: vi.fn().mockResolvedValue({ id: 'new-thread', labels: [] }),
    getThreads: vi.fn().mockResolvedValue([]),
    getMessages: vi.fn().mockResolvedValue([]),
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
vi.mock('../../hooks/useUsageState', () => ({
  // Reads `useCoreState`, which needs a provider this test does not mount.
  useUsageState: () => ({ usage: null, loading: false, error: null, refresh: vi.fn() }),
}));
vi.mock('../../lib/coreState/store', () => ({
  getCoreStateSnapshot: vi.fn(() => ({ isBootstrapping: false, isReady: true })),
  subscribeCoreState: vi.fn(() => () => undefined),
}));

const BACKGROUND_PARK: PendingApproval = {
  request_id: 'req-triage-1',
  tool_name: 'triage.escalate',
  action_summary: 'triage::ESCALATE target=orchestrator prompt_chars=812',
  args_redacted: { action: 'escalate' },
  session_id: 's1',
  created_at: '2026-09-23T04:12:00.000Z',
  expires_at: '2026-09-23T04:22:00.000Z',
};

const NATIVE_SECRET_GRANT: PendingApproval = {
  request_id: 'req-secret-grant-1',
  tool_name: 'native_secret_grant',
  action_summary: 'Unlock configuration secrets',
  args_redacted: null,
  created_at: '2026-10-26T10:00:00.000Z',
  expires_at: '2026-10-26T10:00:30.000Z',
};

async function renderChatRoute() {
  const store = configureStore({
    reducer: combineReducers({
      thread: threadReducer,
      layout: layoutReducer,
      socket: socketReducer,
      chatRuntime: chatRuntimeReducer,
      theme: themeReducer,
    }),
  });
  const { default: Conversations } = await import('../../features/conversations/Conversations');
  render(
    <Provider store={store}>
      <MemoryRouter initialEntries={['/chat']}>
        <SidebarSlotProvider>
          <SidebarSlotOutlet />
          <Routes>
            <Route path="/chat/:threadId?" element={<Conversations />} />
          </Routes>
        </SidebarSlotProvider>
      </MemoryRouter>
    </Provider>
  );
  return store;
}

beforeEach(() => {
  resetFlowPendingApprovalsStoreForTests();
  vi.mocked(decideApproval).mockResolvedValue(undefined as never);
});

afterEach(() => {
  resetFlowPendingApprovalsStoreForTests();
  vi.clearAllMocks();
});

describe('a background approval on /chat', () => {
  it('surfaces a park that has no thread behind it', async () => {
    vi.mocked(fetchPendingApprovals).mockResolvedValue([BACKGROUND_PARK]);

    await renderChatRoute();

    const card = await screen.findByTestId('unrouted-approval-card', undefined, { timeout: 5000 });
    // The tool, so the user can tell what is being asked, not just that
    // something is — rendered as the shared `ApprovalCardAdapter`'s command
    // panel now (the tool name doubles as the command when the gate has no
    // redacted command/path/url of its own).
    expect(card).toHaveTextContent('triage.escalate');
    expect(card).toHaveTextContent('triage::ESCALATE');
  });

  it('records the decision through the shared approval RPC', async () => {
    vi.mocked(fetchPendingApprovals).mockResolvedValue([BACKGROUND_PARK]);
    await renderChatRoute();
    await screen.findByTestId('unrouted-approval-card', undefined, { timeout: 5000 });

    await userEvent.click(screen.getByRole('button', { name: 'Approve' }));

    await waitFor(() =>
      expect(vi.mocked(decideApproval)).toHaveBeenCalledWith('req-triage-1', 'approve_once')
    );
  });

  it('surfaces and approves a native secret grant through the unrouted queue', async () => {
    vi.mocked(fetchPendingApprovals).mockResolvedValue([NATIVE_SECRET_GRANT]);
    await renderChatRoute();

    const card = await screen.findByTestId('unrouted-approval-card', undefined, { timeout: 5000 });
    expect(card).toHaveTextContent('native_secret_grant');
    expect(card).toHaveTextContent('Unlock configuration secrets');

    await userEvent.click(screen.getByRole('button', { name: 'Approve' }));

    await waitFor(() =>
      expect(vi.mocked(decideApproval)).toHaveBeenCalledWith('req-secret-grant-1', 'approve_once')
    );
  });

  it('renders nothing when the queue is empty', async () => {
    // The banner must not be a permanent fixture of the chat surface.
    vi.mocked(fetchPendingApprovals).mockResolvedValue([]);

    await renderChatRoute();

    await waitFor(() => expect(vi.mocked(fetchPendingApprovals)).toHaveBeenCalled());
    expect(screen.queryByTestId('unrouted-approval-card')).toBeNull();
  });
});
