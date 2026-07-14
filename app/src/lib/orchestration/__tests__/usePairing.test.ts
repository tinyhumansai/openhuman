import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { PaymentRequiredError } from '../../agentworld/invokeApiClient';
import { usePairing } from '../usePairing';

const listMock = vi.hoisted(() => vi.fn());
vi.mock('../../../agentworld/AgentWorldShell', () => ({
  apiClient: { orchestrationPairing: { list: listMock } },
}));

const snapshot = {
  records: [],
  contacts: { contacts: [] },
  requests: { incoming: [], outgoing: [] },
  stats: { agentId: 'me', contactCount: 2, pendingIncoming: 1, pendingOutgoing: 0 },
};

describe('usePairing', () => {
  beforeEach(() => vi.clearAllMocks());
  afterEach(() => vi.restoreAllMocks());

  it('loads a snapshot on mount', async () => {
    listMock.mockResolvedValue(snapshot);
    const { result } = renderHook(() => usePairing());
    await waitFor(() => expect(result.current.state.status).toBe('ok'));
    expect(listMock).toHaveBeenCalledTimes(1);
  });

  it('maps a PaymentRequiredError to payment_required', async () => {
    listMock.mockRejectedValue(new PaymentRequiredError({}));
    const { result } = renderHook(() => usePairing());
    await waitFor(() => expect(result.current.state.status).toBe('payment_required'));
  });

  it('surfaces a generic error', async () => {
    listMock.mockRejectedValue(new Error('boom'));
    const { result } = renderHook(() => usePairing());
    await waitFor(() => expect(result.current.state.status).toBe('error'));
  });

  it('runs an action then reloads', async () => {
    listMock.mockResolvedValue(snapshot);
    const { result } = renderHook(() => usePairing());
    await waitFor(() => expect(result.current.state.status).toBe('ok'));
    const action = vi.fn().mockResolvedValue(undefined);
    await act(async () => {
      await result.current.runAction('block:x', action);
    });
    expect(action).toHaveBeenCalledTimes(1);
    expect(listMock).toHaveBeenCalledTimes(2); // initial + reload
    expect(result.current.actionError).toBeNull();
  });

  it('captures an action error without reloading', async () => {
    listMock.mockResolvedValue(snapshot);
    const { result } = renderHook(() => usePairing());
    await waitFor(() => expect(result.current.state.status).toBe('ok'));
    await act(async () => {
      await result.current.runAction('block:x', () => Promise.reject(new Error('nope')));
    });
    expect(result.current.actionError).toBe('nope');
    expect(listMock).toHaveBeenCalledTimes(1); // no reload after failure
  });
});
