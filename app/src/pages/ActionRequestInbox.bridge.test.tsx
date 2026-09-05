/**
 * Bounded UI integration scenario for M1.2.3 ActionRequest inbox.
 *
 * Covers the user-visible path in one journey: approve + reject, storage
 * fail-closed, authoritative Core get, and pending-filter removal.
 *
 * This is the real page against a mocked `createCoreActionRequestClient`
 * interface — not a bridge/client contract proof (the Core RPC client is
 * fully replaced in this suite).
 */
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { setActiveUserId } from '../store/userScopedStorage';
import ActionRequestInbox, { setActionRequestIntentStorageAdapter } from './ActionRequestInbox';

const mockClient = vi.hoisted(() => ({
  list: vi.fn(),
  get: vi.fn(),
  approve: vi.fn(),
  reject: vi.fn(),
}));

vi.mock('../services/api/coreActionRequestClient', async () => {
  const actual = await vi.importActual<typeof import('../services/api/coreActionRequestClient')>(
    '../services/api/coreActionRequestClient'
  );
  return { ...actual, createCoreActionRequestClient: () => mockClient };
});

const PENDING_A = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';
const PENDING_B = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb';
const TENANT = '20000000-0000-0000-0000-000000000001';
const USER = 'bridge-operator';

function envelope(id: string, approval: string, rowVersion: number): Record<string, unknown> {
  return {
    id,
    tenant_id: TENANT,
    row_version: rowVersion,
    approval_state: approval,
    execution_state: 'not_started',
    policy_outcome: 'require_approval',
    correlation_id: `corr-${id.slice(0, 8)}`,
    created_at: '2026-08-08T12:00:00Z',
    updated_at: '2026-08-08T12:00:00Z',
    action_request: {
      id,
      action_type: 'task.escalate',
      risk: 'high',
      proposer: { type: 'agent', id: 'openclaw-main' },
      target: { type: 'task', id: `task-${id.slice(0, 4)}` },
      policy: { reasons: ['needs review'], obligations: [] },
      payload: { summary: `payload-${id.slice(0, 4)}` },
      links: {
        audit_log_ids: [],
        domain_event_ids: [],
        outbox_delivery_ids: [],
        workflow_id: null,
        workflow_trace_id: null,
        agent_run_id: null,
        proposal_event_id: null,
        idempotency_key: null,
      },
    },
  };
}

describe('ActionRequestInbox bridge journey', () => {
  beforeEach(() => {
    mockClient.list.mockReset();
    mockClient.get.mockReset();
    mockClient.approve.mockReset();
    mockClient.reject.mockReset();
    window.localStorage.clear();
    setActionRequestIntentStorageAdapter(null);
    setActiveUserId(USER);
  });

  afterEach(() => {
    setActionRequestIntentStorageAdapter(null);
    setActiveUserId(null);
    vi.restoreAllMocks();
  });

  it('covers approve, reject, storage fail-closed, Core refresh, and pending removal', async () => {
    const pendingA = envelope(PENDING_A, 'pending', 1);
    const pendingB = envelope(PENDING_B, 'pending', 2);
    const approvedA = envelope(PENDING_A, 'approved', 2);
    const rejectedB = envelope(PENDING_B, 'rejected', 3);

    mockClient.list.mockResolvedValue([pendingA, pendingB]);
    mockClient.approve.mockResolvedValueOnce(approvedA);
    mockClient.get.mockResolvedValueOnce(approvedA);
    mockClient.reject.mockResolvedValueOnce(rejectedB);
    mockClient.get.mockResolvedValueOnce(rejectedB);

    const user = userEvent.setup();
    render(<ActionRequestInbox />);

    // --- Approve path: authoritative get + pending-filter removal ---
    expect(await screen.findByTestId(`action-request-row-${PENDING_A}`)).toBeInTheDocument();
    expect(screen.getByTestId(`action-request-row-${PENDING_B}`)).toBeInTheDocument();

    await user.click(screen.getByTestId(`action-request-row-${PENDING_A}`));
    await user.type(screen.getByTestId('action-request-reason'), 'bridge approve ok');
    await user.click(screen.getByTestId('action-request-approve'));

    await waitFor(() => expect(mockClient.approve).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mockClient.get).toHaveBeenCalledWith(PENDING_A));
    await waitFor(() =>
      expect(screen.queryByTestId(`action-request-row-${PENDING_A}`)).not.toBeInTheDocument()
    );
    // B remains pending under the default pending filter.
    expect(screen.getByTestId(`action-request-row-${PENDING_B}`)).toBeInTheDocument();

    // --- Persistence failure blocks Core mutation ---
    setActionRequestIntentStorageAdapter({
      getItem: () => null,
      setItem: () => {
        throw new Error('quota exceeded');
      },
      removeItem: () => undefined,
    });

    await user.click(screen.getByTestId(`action-request-row-${PENDING_B}`));
    await user.clear(screen.getByTestId('action-request-reason'));
    await user.type(screen.getByTestId('action-request-reason'), 'blocked by storage');
    await user.click(screen.getByTestId('action-request-reject'));

    expect(await screen.findByTestId('action-request-error')).toHaveTextContent(
      'retry-key storage'
    );
    expect(mockClient.reject).not.toHaveBeenCalled();

    // Restore durable storage and complete reject + Core refresh + removal.
    setActionRequestIntentStorageAdapter(null);
    await user.clear(screen.getByTestId('action-request-reason'));
    await user.type(screen.getByTestId('action-request-reason'), 'bridge reject ok');
    await user.click(screen.getByTestId('action-request-reject'));

    await waitFor(() => expect(mockClient.reject).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mockClient.get).toHaveBeenCalledWith(PENDING_B));
    await waitFor(() =>
      expect(screen.queryByTestId(`action-request-row-${PENDING_B}`)).not.toBeInTheDocument()
    );
    expect(await screen.findByTestId('action-request-empty')).toBeInTheDocument();

    // Bridge contract: every successful mutation performed authoritative get.
    expect(mockClient.get).toHaveBeenCalledTimes(2);
    expect(mockClient.approve).toHaveBeenCalledTimes(1);
    expect(mockClient.reject).toHaveBeenCalledTimes(1);
  });
});
