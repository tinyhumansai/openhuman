/**
 * `onRespondToToolApproval` — the adapter callback assistant-ui calls when a
 * decision is made on a gated tool part.
 *
 * Supplying it at all is load-bearing: without it the runtime *throws*
 * "Runtime does not support tool approvals." rather than no-opping, so a
 * decision button becomes a crash. `AssistantUiRuntimeProvider.approvalGate`
 * covers that happy path — a decision reaches `openhuman.approval_decide` and
 * the gate clears.
 *
 * Two rules it encodes have no test that fails when they are removed, and both
 * decide whether a thread can be un-blocked at all:
 *
 *  1. the optimistic clear happens ONLY after the RPC resolves. Clearing first
 *     (or in a `finally`) on a decide that failed drops the last prompt on
 *     screen while the core is still parked — the turn then hangs until the
 *     gate's 10-minute TTL with nothing left to retry from. `ApprovalRequestCard`
 *     has the same rule and its own test for it; this is the *other* live
 *     decision path, the one that runs inside the runtime.
 *  2. a renderer that answers with the plain `approved` boolean rather than
 *     picking one of the declared options still maps to a decision the core
 *     accepts — `approve_once` / `deny`, not `undefined`.
 */
import { configureStore } from '@reduxjs/toolkit';
import { renderHook, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../services/coreRpcClient';
import chatRuntimeReducer, {
  type PendingApproval,
  setPendingApprovalForThread,
} from '../../store/chatRuntimeSlice';
import threadReducer from '../../store/threadSlice';
import { useOpenHumanExternalStore } from '../useOpenHumanExternalStore';

vi.mock('../../services/api/threadApi', () => ({
  threadApi: {
    getDerivedTranscript: vi
      .fn()
      .mockResolvedValue({
        threadId: 't-decide',
        items: [],
        total: 0,
        hasMore: false,
        hasTranscript: false,
      }),
  },
}));

vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const THREAD_ID = 't-decide';
const REQUEST_ID = 'appr-9';

const GATE: PendingApproval = {
  requestId: REQUEST_ID,
  toolName: 'shell',
  message: 'Run shell — delete the build directory',
  command: 'rm -rf ./dist',
};

function buildStore() {
  const store = configureStore({
    reducer: { thread: threadReducer, chatRuntime: chatRuntimeReducer },
  });
  store.dispatch(setPendingApprovalForThread({ threadId: THREAD_ID, approval: GATE }));
  return store;
}

function parkedGate(store: ReturnType<typeof buildStore>) {
  return store.getState().chatRuntime.pendingApprovalByThread[THREAD_ID];
}

function mountAdapter(store: ReturnType<typeof buildStore>) {
  const wrapper = ({ children }: { children: ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  return renderHook(() => useOpenHumanExternalStore(THREAD_ID), { wrapper });
}

describe('answering the tool-approval gate through the runtime', () => {
  beforeEach(() => {
    vi.mocked(callCoreRpc).mockReset();
  });

  it('keeps the gate parked when the decide never lands', async () => {
    vi.mocked(callCoreRpc).mockRejectedValue(new Error('core unreachable'));
    const store = buildStore();
    const { result } = mountAdapter(store);
    expect(parkedGate(store)).toBeDefined();

    await expect(
      result.current.onRespondToToolApproval({
        approvalId: REQUEST_ID,
        approved: true,
        optionId: 'approve_once',
      })
    ).rejects.toThrow('core unreachable');

    // The core is still holding the call, so the prompt must still be there to
    // answer. Clearing here is what strands the thread until the gate expires.
    expect(parkedGate(store)).toEqual(GATE);
  });

  it('clears the gate once the decide has landed', async () => {
    // The other side of the same rule, so a fix that simply never clears is
    // not mistaken for a fix.
    vi.mocked(callCoreRpc).mockResolvedValue(undefined);
    const store = buildStore();
    const { result } = mountAdapter(store);

    await result.current.onRespondToToolApproval({
      approvalId: REQUEST_ID,
      approved: true,
      optionId: 'approve_once',
    });

    await waitFor(() => expect(parkedGate(store)).toBeUndefined());
  });

  it('maps a bare approved/denied answer onto a decision the core accepts', async () => {
    vi.mocked(callCoreRpc).mockResolvedValue(undefined);
    const store = buildStore();
    const { result } = mountAdapter(store);

    await result.current.onRespondToToolApproval({ approvalId: REQUEST_ID, approved: true });
    expect(callCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.approval_decide',
      params: { request_id: REQUEST_ID, decision: 'approve_once' },
    });

    await result.current.onRespondToToolApproval({ approvalId: REQUEST_ID, approved: false });
    expect(callCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.approval_decide',
      params: { request_id: REQUEST_ID, decision: 'deny' },
    });
  });
});
