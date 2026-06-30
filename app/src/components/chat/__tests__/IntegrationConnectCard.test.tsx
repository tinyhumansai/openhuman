import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { authorize, listConnections } from '../../../lib/composio/composioApi';
import { deriveComposioState } from '../../../lib/composio/types';
import { callCoreRpc } from '../../../services/coreRpcClient';
import chatRuntimeReducer, {
  type PendingApproval,
  setPendingApprovalForThread,
} from '../../../store/chatRuntimeSlice';
import { openUrl } from '../../../utils/openUrl';
import IntegrationConnectCard from '../IntegrationConnectCard';

vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));
vi.mock('../../../utils/openUrl', () => ({ openUrl: vi.fn() }));
vi.mock('../../../lib/composio/composioApi', () => ({
  authorize: vi.fn(),
  listConnections: vi.fn(),
}));
vi.mock('../../../lib/composio/types', () => ({ deriveComposioState: vi.fn() }));

const THREAD = 't1';
const approval: PendingApproval = {
  requestId: 'req-connect-1',
  toolName: 'composio_connect',
  message: 'Connect gmail to complete your task',
  toolkit: 'gmail',
};

function renderCard() {
  const store = configureStore({ reducer: { chatRuntime: chatRuntimeReducer } });
  store.dispatch(setPendingApprovalForThread({ threadId: THREAD, approval }));
  const utils = render(
    <Provider store={store}>
      <IntegrationConnectCard threadId={THREAD} approval={approval} />
    </Provider>
  );
  return { store, ...utils };
}

describe('IntegrationConnectCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the connect prompt, Connect button, and tool name', () => {
    renderCard();
    // The action message leads the card (no alarming "Approval needed" title).
    expect(screen.getByText('Connect gmail to complete your task')).toBeInTheDocument();
    expect(screen.getByText('Connect')).toBeInTheDocument();
    expect(screen.getByText('composio_connect')).toBeInTheDocument();
  });

  it('Connect authorizes, opens the OAuth url, polls, and resolves approve_once on active', async () => {
    vi.mocked(authorize).mockResolvedValueOnce({
      connectUrl: 'https://hosted.composio.dev/abc',
      connectionId: 'conn-1',
    } as Awaited<ReturnType<typeof authorize>>);
    vi.mocked(listConnections).mockResolvedValue({
      connections: [{ toolkit: 'gmail', status: 'ACTIVE' }],
    } as Awaited<ReturnType<typeof listConnections>>);
    // First poll tick sees the toolkit ACTIVE.
    vi.mocked(deriveComposioState).mockReturnValue('connected');
    vi.mocked(callCoreRpc).mockResolvedValue({});

    const { store } = renderCard();
    fireEvent.click(screen.getByText('Connect'));

    // No required fields for gmail → authorize called with no extra params.
    await waitFor(() => expect(authorize).toHaveBeenCalledWith('gmail', undefined));
    await waitFor(() => expect(openUrl).toHaveBeenCalledWith('https://hosted.composio.dev/abc'));
    // Polling detected the live connection → parked tool call resolved as approved.
    await waitFor(() =>
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.approval_decide',
        params: { request_id: 'req-connect-1', decision: 'approve_once' },
      })
    );
    await waitFor(() =>
      expect(store.getState().chatRuntime.pendingApprovalByThread[THREAD]).toBeUndefined()
    );
  });

  it('Cancel resolves the parked call as deny', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({});
    const { store } = renderCard();

    fireEvent.click(screen.getByText('Deny'));

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.approval_decide',
      params: { request_id: 'req-connect-1', decision: 'deny' },
    });
    await waitFor(() =>
      expect(store.getState().chatRuntime.pendingApprovalByThread[THREAD]).toBeUndefined()
    );
  });

  it('keeps the card mounted and surfaces an error when approval_decide fails', async () => {
    // The decide RPC throws — the backend request is still parked, so clearing
    // the card would strand the thread until the gate TTL expires. The card
    // must stay (so the user can retry/deny) and surface the failure (#4062).
    vi.mocked(callCoreRpc).mockRejectedValueOnce(new Error('rpc down'));
    const { store } = renderCard();

    fireEvent.click(screen.getByText('Deny'));

    await waitFor(() =>
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.approval_decide',
        params: { request_id: 'req-connect-1', decision: 'deny' },
      })
    );
    // The parked approval survives the failed decide — not cleared.
    expect(store.getState().chatRuntime.pendingApprovalByThread[THREAD]).toBeDefined();
    // The failure is shown rather than silently swallowed.
    await waitFor(() =>
      expect(screen.getByText(/Could not record your decision/)).toBeInTheDocument()
    );
  });

  it('collects required fields inline before authorizing (whatsapp waba_id)', async () => {
    vi.mocked(authorize).mockResolvedValue({
      connectUrl: 'https://hosted.composio.dev/wa',
      connectionId: 'conn-wa',
    } as Awaited<ReturnType<typeof authorize>>);
    // Not connected yet — poll keeps waiting; we only assert the authorize args.
    vi.mocked(listConnections).mockResolvedValue({ connections: [] } as Awaited<
      ReturnType<typeof listConnections>
    >);

    const waApproval: PendingApproval = {
      requestId: 'req-wa',
      toolName: 'composio_connect',
      message: 'Connect whatsapp to complete your task',
      toolkit: 'whatsapp',
    };
    const store = configureStore({ reducer: { chatRuntime: chatRuntimeReducer } });
    store.dispatch(setPendingApprovalForThread({ threadId: THREAD, approval: waApproval }));
    render(
      <Provider store={store}>
        <IntegrationConnectCard threadId={THREAD} approval={waApproval} />
      </Provider>
    );

    // The required field is rendered inline.
    expect(screen.getByText('WhatsApp Business Account ID (WABA ID)')).toBeInTheDocument();

    // Connecting without filling it blocks authorize and shows a field error.
    fireEvent.click(screen.getByText('Connect'));
    expect(authorize).not.toHaveBeenCalled();
    expect(screen.getByText('This field is required.')).toBeInTheDocument();

    // Filling it forwards the value as an extra_param to authorize.
    fireEvent.change(screen.getByRole('textbox'), { target: { value: '123456789012345' } });
    fireEvent.click(screen.getByText('Connect'));
    await waitFor(() =>
      expect(authorize).toHaveBeenCalledWith('whatsapp', { waba_id: '123456789012345' })
    );
  });

  it('canonicalizes the toolkit slug before authorizing (google_drive → googledrive)', async () => {
    vi.mocked(authorize).mockResolvedValueOnce({
      connectUrl: 'https://hosted.composio.dev/gd',
      connectionId: 'conn-gd',
    } as Awaited<ReturnType<typeof authorize>>);
    vi.mocked(listConnections).mockResolvedValue({ connections: [] } as Awaited<
      ReturnType<typeof listConnections>
    >);

    const gdApproval: PendingApproval = {
      requestId: 'req-gd',
      toolName: 'composio_connect',
      message: 'Connect googledrive to complete your task',
      toolkit: 'google_drive',
    };
    const store = configureStore({ reducer: { chatRuntime: chatRuntimeReducer } });
    store.dispatch(setPendingApprovalForThread({ threadId: THREAD, approval: gdApproval }));
    render(
      <Provider store={store}>
        <IntegrationConnectCard threadId={THREAD} approval={gdApproval} />
      </Provider>
    );

    fireEvent.click(screen.getByText('Connect'));
    // The card hits the canonical Composio slug, not the agent's guess.
    await waitFor(() => expect(authorize).toHaveBeenCalledWith('googledrive', undefined));
  });

  it('shows an error and a Retry affordance when authorize fails', async () => {
    vi.mocked(authorize).mockRejectedValueOnce(new Error('backend down'));
    renderCard();

    fireEvent.click(screen.getByText('Connect'));

    // Raw error text is not surfaced; the localized connection-failed string is.
    await waitFor(() => expect(screen.getByText('Retry connection')).toBeInTheDocument());
    // The parked call is NOT resolved on a local authorize failure — the user
    // can retry without the agent giving up.
    expect(callCoreRpc).not.toHaveBeenCalled();
  });

  it('surfaces the backend reason and drops Retry on a permanent rejection', async () => {
    vi.mocked(authorize).mockRejectedValueOnce(
      new Error(
        '[composio] authorize failed: Backend returned 400 Bad Request: No auth config found for toolkit "googledrive"'
      )
    );
    renderCard();

    fireEvent.click(screen.getByText('Connect'));

    // The actual backend reason is shown (diagnosable), not a bare "failed".
    await waitFor(() => expect(screen.getByText(/No auth config found/)).toBeInTheDocument());
    // Retry is gone (it would loop); only Dismiss remains.
    expect(screen.queryByText('Retry connection')).not.toBeInTheDocument();
    expect(screen.getByText('Deny')).toBeInTheDocument();
  });

  it('surfaces the status when polling finds an errored connection', async () => {
    vi.mocked(authorize).mockResolvedValueOnce({
      connectUrl: 'https://hosted.composio.dev/e',
      connectionId: 'conn-e',
    } as Awaited<ReturnType<typeof authorize>>);
    vi.mocked(listConnections).mockResolvedValue({
      connections: [{ toolkit: 'gmail', status: 'FAILED' }],
    } as Awaited<ReturnType<typeof listConnections>>);
    vi.mocked(deriveComposioState).mockReturnValue('error');

    renderCard();
    fireEvent.click(screen.getByText('Connect'));

    await waitFor(() => expect(screen.getByText(/Connection failed/)).toBeInTheDocument());
    // A poll-detected error leaves the card for the user to retry/dismiss;
    // it does NOT auto-resolve the gate.
    expect(callCoreRpc).not.toHaveBeenCalled();
  });

  it('shows "additional config required" when authorize reports missing fields for a field-less toolkit', async () => {
    vi.mocked(authorize).mockRejectedValueOnce(
      new Error('400: ConnectedAccount_MissingRequiredFields')
    );
    renderCard(); // gmail has no entry in the required-fields registry

    fireEvent.click(screen.getByText('Connect'));

    // Error line renders as "⚠ {msg}", so match the substring.
    await waitFor(() => expect(screen.getByText(/Additional config required/)).toBeInTheDocument());
  });

  it('approves when any matching row is ACTIVE even behind a stale FAILED row', async () => {
    vi.mocked(authorize).mockResolvedValueOnce({
      connectUrl: 'https://hosted.composio.dev/m',
      connectionId: 'conn-m',
    } as Awaited<ReturnType<typeof authorize>>);
    // First row is an old FAILED handoff; the freshly-authorized row is ACTIVE.
    vi.mocked(listConnections).mockResolvedValue({
      connections: [
        { toolkit: 'gmail', status: 'FAILED' },
        { toolkit: 'gmail', status: 'ACTIVE' },
      ],
    } as Awaited<ReturnType<typeof listConnections>>);
    vi.mocked(deriveComposioState).mockImplementation((c?: { status: string }) =>
      c?.status === 'ACTIVE' ? 'connected' : 'error'
    );
    vi.mocked(callCoreRpc).mockResolvedValue({});

    renderCard();
    fireEvent.click(screen.getByText('Connect'));

    // The ACTIVE row wins over the stale FAILED row → approve.
    await waitFor(() =>
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.approval_decide',
        params: { request_id: 'req-connect-1', decision: 'approve_once' },
      })
    );
  });

  it('aborts the authorize continuation if the card is dismissed mid-flight', async () => {
    let resolveAuthorize!: (v: Awaited<ReturnType<typeof authorize>>) => void;
    vi.mocked(authorize).mockReturnValueOnce(
      new Promise(resolve => {
        resolveAuthorize = resolve;
      })
    );
    vi.mocked(callCoreRpc).mockResolvedValue({});

    renderCard();
    fireEvent.click(screen.getByText('Connect'));
    // Deny while authorize is still in flight.
    fireEvent.click(screen.getByText('Deny'));
    await waitFor(() =>
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.approval_decide',
        params: { request_id: 'req-connect-1', decision: 'deny' },
      })
    );

    // authorize finally resolves — the continuation must NOT open OAuth.
    resolveAuthorize({
      connectUrl: 'https://hosted.composio.dev/x',
      connectionId: 'conn-x',
    } as Awaited<ReturnType<typeof authorize>>);
    await Promise.resolve();
    await Promise.resolve();
    expect(openUrl).not.toHaveBeenCalled();
  });

  it('resolves the gate as deny when the OAuth poll times out', async () => {
    vi.useFakeTimers();
    try {
      vi.mocked(authorize).mockResolvedValueOnce({
        connectUrl: 'https://hosted.composio.dev/t',
        connectionId: 'conn-t',
      } as Awaited<ReturnType<typeof authorize>>);
      // Never connects → poll runs until the 5-min deadline.
      vi.mocked(listConnections).mockResolvedValue({ connections: [] } as Awaited<
        ReturnType<typeof listConnections>
      >);
      vi.mocked(callCoreRpc).mockResolvedValue({});

      renderCard();
      fireEvent.click(screen.getByText('Connect'));

      // Flush authorize + run polling past the 5-min deadline.
      await vi.advanceTimersByTimeAsync(5 * 60 * 1000 + 5000);

      // Timeout resolves the parked tool call as deny so the agent resumes.
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.approval_decide',
        params: { request_id: 'req-connect-1', decision: 'deny' },
      });
    } finally {
      vi.useRealTimers();
    }
  });
});
