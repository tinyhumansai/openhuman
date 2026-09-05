import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { setActiveUserId } from '../store/userScopedStorage';
import ActionRequestInbox, {
  actionRequestIdempotencyStorageKey,
  clearAllDecisionIdempotencyKeys,
  clearIdempotencyKey,
  fingerprintReason,
  getOrCreateIdempotencyKey,
  type IntentScope,
  type IntentStorageAdapter,
  resolveActiveUserScope,
  setActionRequestIntentStorageAdapter,
} from './ActionRequestInbox';

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

const PENDING_ID = '33333333-3333-4333-8333-333333333333';
const TENANT_ID = '20000000-0000-0000-0000-000000000001';
const ACTIVE_USER = 'test-active-user';

const pendingItem = {
  action_request: {
    id: PENDING_ID,
    action_type: 'task.escalate',
    risk: 'high',
    proposer: { type: 'agent', id: 'openclaw-main' },
    target: { type: 'task', id: 'task-1' },
    policy: {
      reasons: ['high risk requires human approval'],
      obligations: ['notify owner after decision'],
    },
    payload: { summary: 'Escalate missed check-in' },
    links: {
      workflow_id: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
      workflow_trace_id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',
      agent_run_id: 'cccccccc-cccc-4ccc-8ccc-cccccccccccc',
      proposal_event_id: 'proposal-evt-1',
      idempotency_key: 'create-key-1',
      audit_log_ids: ['dddddddd-dddd-4ddd-8ddd-dddddddddddd'],
      domain_event_ids: ['domain-evt-1'],
      outbox_delivery_ids: ['eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee'],
    },
  },
  row_version: 2,
  id: PENDING_ID,
  tenant_id: TENANT_ID,
  approval_state: 'pending',
  execution_state: 'not_started',
  policy_outcome: 'require_approval',
  correlation_id: 'corr_1',
  created_at: '2026-08-08T12:00:00Z',
  updated_at: '2026-08-08T12:00:00Z',
} as const;

function defaultScope(): IntentScope {
  return { tenantId: TENANT_ID, activeUserId: ACTIVE_USER };
}

/** Physical key under default user-scoped verified adapter. */
function physicalStorageKey(tenantId = TENANT_ID, userId = ACTIVE_USER) {
  return `${userId}:${actionRequestIdempotencyStorageKey(tenantId)}`;
}

function storedIdempotencyRecords() {
  const raw = window.localStorage.getItem(physicalStorageKey());
  return raw
    ? (JSON.parse(raw) as Record<
        string,
        { key: string; reasonFingerprint?: string; reason?: string }
      >)
    : {};
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, resolve, reject };
}

function structuredError(code: string, field?: string) {
  return { data: { kind: 'YouPetCoreHttpError', youpet: field ? { code, field } : { code } } };
}

function memoryStorageAdapter(initial: Record<string, string> = {}): IntentStorageAdapter {
  const map = new Map<string, string>(Object.entries(initial));
  return {
    getItem(key) {
      return map.has(key) ? (map.get(key) as string) : null;
    },
    setItem(key, value) {
      map.set(key, value);
    },
    removeItem(key) {
      map.delete(key);
    },
  };
}

describe('ActionRequestInbox', () => {
  beforeEach(() => {
    mockClient.list.mockReset();
    mockClient.get.mockReset();
    mockClient.approve.mockReset();
    mockClient.reject.mockReset();
    window.localStorage.clear();
    setActionRequestIntentStorageAdapter(null);
    setActiveUserId(ACTIVE_USER);
  });

  afterEach(() => {
    setActionRequestIntentStorageAdapter(null);
    setActiveUserId(null);
    vi.restoreAllMocks();
  });

  it('renders pending request context including links for operator review', async () => {
    mockClient.list.mockResolvedValueOnce([pendingItem]);

    render(<ActionRequestInbox />);

    expect(await screen.findByTestId(`action-request-row-${PENDING_ID}`)).toBeInTheDocument();
    expect(screen.getByTestId('action-request-detail-id')).toHaveTextContent(PENDING_ID);
    expect(screen.getByTestId('action-request-detail')).toHaveTextContent('task.escalate');
    expect(screen.getByText(/agent · openclaw-main/)).toBeInTheDocument();
    expect(screen.getByText(/task · task-1/)).toBeInTheDocument();
    expect(screen.getByText('high risk requires human approval')).toBeInTheDocument();
    expect(screen.getByTestId('action-request-link-workflow')).toHaveTextContent(
      'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'
    );
    expect(screen.getByTestId('action-request-link-proposal')).toHaveTextContent('proposal-evt-1');
    expect(screen.getByTestId('action-request-link-audit')).toHaveTextContent(
      'dddddddd-dddd-4ddd-8ddd-dddddddddddd'
    );
    expect(screen.getByTestId('action-request-approve')).toBeInTheDocument();
    expect(screen.getByTestId('action-request-reject')).toBeInTheDocument();
  });

  it('shows empty, loading, and error states including tenant config', async () => {
    mockClient.list.mockResolvedValueOnce([]);
    const first = render(<ActionRequestInbox />);
    expect(await screen.findByTestId('action-request-empty')).toBeInTheDocument();
    first.unmount();

    const pendingList = deferred<(typeof pendingItem)[]>();
    mockClient.list.mockReturnValueOnce(pendingList.promise);
    const loadingRender = render(<ActionRequestInbox />);
    expect(await screen.findByTestId('action-request-loading')).toBeInTheDocument();
    pendingList.resolve([]);
    await waitFor(() =>
      expect(screen.queryByTestId('action-request-loading')).not.toBeInTheDocument()
    );
    loadingRender.unmount();

    mockClient.list.mockRejectedValueOnce({
      data: { kind: 'YouPetConfigMissing', youpet: { field: 'tenant_id' } },
    });
    render(<ActionRequestInbox />);
    expect(await screen.findByTestId('action-request-error')).toHaveTextContent('YOUPET_TENANT_ID');
  });

  it('renders empty links when Core omits correlation links', async () => {
    mockClient.list.mockResolvedValueOnce([
      {
        ...pendingItem,
        action_request: {
          ...pendingItem.action_request,
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
      },
    ]);
    render(<ActionRequestInbox />);
    expect(await screen.findByTestId('action-request-links-empty')).toBeInTheDocument();
  });

  it('approves with reason, row version, stable key, Core refresh, and pending-filter removal', async () => {
    const approved = { ...pendingItem, approval_state: 'approved', row_version: 3 };
    mockClient.list.mockResolvedValue([pendingItem]);
    mockClient.approve.mockResolvedValueOnce(approved);
    mockClient.get.mockResolvedValueOnce(approved);
    const user = userEvent.setup();

    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.type(screen.getByTestId('action-request-reason'), 'safe to proceed');
    await user.click(screen.getByTestId('action-request-approve'));

    await waitFor(() => expect(mockClient.approve).toHaveBeenCalledTimes(1));
    expect(mockClient.approve).toHaveBeenCalledWith(PENDING_ID, {
      reason: 'safe to proceed',
      expectedRowVersion: 2,
      idempotencyKey: expect.stringContaining(`youpet-action-request:approve:${PENDING_ID}:`),
    });
    await waitFor(() => expect(mockClient.get).toHaveBeenCalledWith(PENDING_ID));
    expect(storedIdempotencyRecords()[`approve:${PENDING_ID}`]).toBeUndefined();
    expect(storedIdempotencyRecords()[`reject:${PENDING_ID}`]).toBeUndefined();
    expect(await screen.findByTestId('action-request-empty')).toBeInTheDocument();
    expect(screen.queryByTestId(`action-request-row-${PENDING_ID}`)).not.toBeInTheDocument();
  });

  it('rejects with reason, expected row version, and Core refresh', async () => {
    const rejected = { ...pendingItem, approval_state: 'rejected', row_version: 3 };
    mockClient.list.mockResolvedValue([pendingItem]);
    mockClient.reject.mockResolvedValueOnce(rejected);
    mockClient.get.mockResolvedValueOnce(rejected);
    const user = userEvent.setup();

    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.type(screen.getByTestId('action-request-reason'), 'too risky');
    await user.click(screen.getByTestId('action-request-reject'));

    await waitFor(() => expect(mockClient.reject).toHaveBeenCalledTimes(1));
    expect(mockClient.reject).toHaveBeenCalledWith(PENDING_ID, {
      reason: 'too risky',
      expectedRowVersion: 2,
      idempotencyKey: expect.stringContaining(`youpet-action-request:reject:${PENDING_ID}:`),
    });
    await waitFor(() => expect(mockClient.get).toHaveBeenCalledWith(PENDING_ID));
    expect(storedIdempotencyRecords()[`reject:${PENDING_ID}`]).toBeUndefined();
  });

  it('reuses the same approve idempotency key across failed retry and remount', async () => {
    mockClient.list.mockResolvedValue([pendingItem]);
    mockClient.approve.mockRejectedValue(new Error('temporary failure'));
    const user = userEvent.setup();

    const firstRender = render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.type(screen.getByTestId('action-request-reason'), 'retry me');
    await user.click(screen.getByTestId('action-request-approve'));
    await waitFor(() => expect(mockClient.approve).toHaveBeenCalledTimes(1));
    const firstKey = mockClient.approve.mock.calls[0]?.[1]?.idempotencyKey as string;
    expect(storedIdempotencyRecords()[`approve:${PENDING_ID}`]?.key).toBe(firstKey);
    expect(storedIdempotencyRecords()[`approve:${PENDING_ID}`]?.reasonFingerprint).toBe(
      fingerprintReason('retry me')
    );
    expect(storedIdempotencyRecords()[`approve:${PENDING_ID}`]?.reason).toBeUndefined();

    firstRender.unmount();
    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.type(screen.getByTestId('action-request-reason'), 'retry me');
    await user.click(screen.getByTestId('action-request-approve'));
    await waitFor(() => expect(mockClient.approve).toHaveBeenCalledTimes(2));
    expect(mockClient.approve.mock.calls[1]?.[1]?.idempotencyKey).toBe(firstKey);
  });

  it('reuses the same reject idempotency key across failed retry and remount', async () => {
    mockClient.list.mockResolvedValue([pendingItem]);
    mockClient.reject.mockRejectedValue(new Error('temporary failure'));
    const user = userEvent.setup();

    const firstRender = render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.type(screen.getByTestId('action-request-reason'), 'reject retry');
    await user.click(screen.getByTestId('action-request-reject'));
    await waitFor(() => expect(mockClient.reject).toHaveBeenCalledTimes(1));
    const firstKey = mockClient.reject.mock.calls[0]?.[1]?.idempotencyKey as string;

    firstRender.unmount();
    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.type(screen.getByTestId('action-request-reason'), 'reject retry');
    await user.click(screen.getByTestId('action-request-reject'));
    await waitFor(() => expect(mockClient.reject).toHaveBeenCalledTimes(2));
    expect(mockClient.reject.mock.calls[1]?.[1]?.idempotencyKey).toBe(firstKey);
  });

  it('rotates the key when the operator reason changes after failure', async () => {
    mockClient.list.mockResolvedValue([pendingItem]);
    mockClient.approve.mockRejectedValue(new Error('temporary failure'));
    const user = userEvent.setup();

    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.type(screen.getByTestId('action-request-reason'), 'first reason');
    await user.click(screen.getByTestId('action-request-approve'));
    await waitFor(() => expect(mockClient.approve).toHaveBeenCalledTimes(1));
    const firstKey = mockClient.approve.mock.calls[0]?.[1]?.idempotencyKey as string;

    await user.clear(screen.getByTestId('action-request-reason'));
    await user.type(screen.getByTestId('action-request-reason'), 'changed reason');
    await user.click(screen.getByTestId('action-request-approve'));
    await waitFor(() => expect(mockClient.approve).toHaveBeenCalledTimes(2));
    const secondKey = mockClient.approve.mock.calls[1]?.[1]?.idempotencyKey as string;
    expect(secondKey).not.toBe(firstKey);
    expect(storedIdempotencyRecords()[`approve:${PENDING_ID}`]?.reasonFingerprint).toBe(
      fingerprintReason('changed reason')
    );
  });

  it('does not reuse an approve key for reject', async () => {
    mockClient.list.mockResolvedValue([pendingItem]);
    mockClient.approve.mockRejectedValueOnce(new Error('temporary failure'));
    mockClient.reject.mockRejectedValueOnce(new Error('temporary failure'));
    const user = userEvent.setup();

    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.type(screen.getByTestId('action-request-reason'), 'decision reason');
    await user.click(screen.getByTestId('action-request-approve'));
    await waitFor(() => expect(mockClient.approve).toHaveBeenCalledTimes(1));
    const approveKey = mockClient.approve.mock.calls[0]?.[1]?.idempotencyKey as string;

    await user.click(screen.getByTestId('action-request-reject'));
    await waitFor(() => expect(mockClient.reject).toHaveBeenCalledTimes(1));
    const rejectKey = mockClient.reject.mock.calls[0]?.[1]?.idempotencyKey as string;
    expect(rejectKey).not.toBe(approveKey);
  });

  it('suppresses duplicate in-flight approve and reject clicks', async () => {
    const approved = { ...pendingItem, approval_state: 'approved', row_version: 3 };
    const pendingApprove = deferred<typeof approved>();
    mockClient.list.mockResolvedValue([pendingItem]);
    mockClient.approve.mockReturnValueOnce(pendingApprove.promise);
    mockClient.get.mockResolvedValue(approved);
    const user = userEvent.setup();

    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.type(screen.getByTestId('action-request-reason'), 'one click only');
    const approveButton = screen.getByTestId('action-request-approve');
    const rejectButton = screen.getByTestId('action-request-reject');

    await user.click(approveButton);
    await waitFor(() => expect(approveButton).toBeDisabled());
    expect(rejectButton).toBeDisabled();
    await user.click(approveButton);
    await user.click(rejectButton);
    expect(mockClient.approve).toHaveBeenCalledTimes(1);
    expect(mockClient.reject).not.toHaveBeenCalled();

    pendingApprove.resolve(approved);
    await waitFor(() =>
      expect(screen.queryByTestId(`action-request-row-${PENDING_ID}`)).not.toBeInTheDocument()
    );
  });

  it('reloads Core state on concurrency conflict and explains the conflict', async () => {
    const refreshed = { ...pendingItem, approval_state: 'approved', row_version: 4 };
    mockClient.list.mockResolvedValue([pendingItem]);
    mockClient.approve.mockRejectedValueOnce(structuredError('concurrency_conflict'));
    mockClient.get.mockResolvedValueOnce(refreshed);
    const user = userEvent.setup();

    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.type(screen.getByTestId('action-request-reason'), 'race');
    await user.click(screen.getByTestId('action-request-approve'));

    await waitFor(() => expect(mockClient.get).toHaveBeenCalledWith(PENDING_ID));
    expect(await screen.findByTestId('action-request-error')).toHaveTextContent(
      'concurrency_conflict'
    );
    expect(screen.getByTestId('action-request-error')).toHaveTextContent('approved');
    expect(screen.getByTestId('action-request-error')).toHaveTextContent('v4');
    await waitFor(() =>
      expect(screen.queryByTestId(`action-request-row-${PENDING_ID}`)).not.toBeInTheDocument()
    );
    expect(storedIdempotencyRecords()[`approve:${PENDING_ID}`]).toBeUndefined();
  });

  it('handles idempotency_conflict with authoritative refresh', async () => {
    const refreshed = { ...pendingItem, approval_state: 'rejected', row_version: 5 };
    mockClient.list.mockResolvedValue([pendingItem]);
    mockClient.reject.mockRejectedValueOnce(structuredError('idempotency_conflict'));
    mockClient.get.mockResolvedValueOnce(refreshed);
    const user = userEvent.setup();

    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.type(screen.getByTestId('action-request-reason'), 'conflict path');
    await user.click(screen.getByTestId('action-request-reject'));

    await waitFor(() => expect(mockClient.get).toHaveBeenCalledWith(PENDING_ID));
    expect(await screen.findByTestId('action-request-error')).toHaveTextContent(
      'idempotency_conflict'
    );
  });

  it('keeps the same-intent key when conflict leaves the row pending', async () => {
    const stillPending = { ...pendingItem, row_version: 3 };
    mockClient.list.mockResolvedValue([pendingItem]);
    mockClient.approve.mockRejectedValueOnce(structuredError('concurrency_conflict'));
    mockClient.get.mockResolvedValueOnce(stillPending);
    const user = userEvent.setup();

    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.type(screen.getByTestId('action-request-reason'), 'retry later');
    await user.click(screen.getByTestId('action-request-approve'));

    await waitFor(() => expect(mockClient.get).toHaveBeenCalledWith(PENDING_ID));
    const approved = { ...stillPending, approval_state: 'approved', row_version: 4 };
    mockClient.approve.mockResolvedValueOnce(approved);
    mockClient.get.mockResolvedValueOnce(approved);
    await user.click(screen.getByTestId('action-request-approve'));
    await waitFor(() => expect(mockClient.approve).toHaveBeenCalledTimes(2));
    expect(mockClient.approve.mock.calls[1]?.[1]?.expectedRowVersion).toBe(3);
  });

  it('hides decision controls for terminal approvals including expired', async () => {
    mockClient.list.mockResolvedValueOnce([
      { ...pendingItem, approval_state: 'expired', row_version: 5 },
    ]);

    render(<ActionRequestInbox />);
    expect(await screen.findByTestId('action-request-terminal')).toBeInTheDocument();
    expect(screen.queryByTestId('action-request-approve')).not.toBeInTheDocument();
  });

  it('requires a non-empty reason before submitting', async () => {
    mockClient.list.mockResolvedValue([pendingItem]);
    const user = userEvent.setup();

    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.click(screen.getByTestId('action-request-approve'));

    expect(await screen.findByTestId('action-request-error')).toHaveTextContent(
      'non-empty operator reason'
    );
    expect(mockClient.approve).not.toHaveBeenCalled();
  });

  it('maps not-found and validation-style Core codes into stable errors', async () => {
    mockClient.list.mockRejectedValueOnce(structuredError('not_found'));
    const first = render(<ActionRequestInbox />);
    expect(await screen.findByTestId('action-request-error')).toHaveTextContent('not_found');
    first.unmount();

    mockClient.list.mockResolvedValue([pendingItem]);
    mockClient.approve.mockRejectedValueOnce(structuredError('invalid_request'));
    const user = userEvent.setup();
    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.type(screen.getByTestId('action-request-reason'), 'validate');
    await user.click(screen.getByTestId('action-request-approve'));
    expect(await screen.findByTestId('action-request-error')).toHaveTextContent('invalid_request');
  });

  it('maps forbidden_consumer_operation Core responses into a stable error', async () => {
    mockClient.list.mockResolvedValue([pendingItem]);
    mockClient.approve.mockRejectedValueOnce(structuredError('forbidden_consumer_operation'));
    const user = userEvent.setup();

    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.type(screen.getByTestId('action-request-reason'), 'not allowed');
    await user.click(screen.getByTestId('action-request-approve'));

    expect(await screen.findByTestId('action-request-error')).toHaveTextContent(
      'forbidden_consumer_operation'
    );
    expect(mockClient.get).not.toHaveBeenCalled();
  });

  it('fails closed when the initial idempotency key cannot be persisted', async () => {
    mockClient.list.mockResolvedValue([pendingItem]);
    const user = userEvent.setup();

    setActionRequestIntentStorageAdapter({
      getItem: () => null,
      setItem: () => {
        throw new Error('quota exceeded');
      },
      removeItem: () => undefined,
    });

    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.type(screen.getByTestId('action-request-reason'), 'blocked write');
    await user.click(screen.getByTestId('action-request-approve'));

    expect(await screen.findByTestId('action-request-error')).toHaveTextContent(
      'retry-key storage'
    );
    expect(mockClient.approve).not.toHaveBeenCalled();
    expect(mockClient.reject).not.toHaveBeenCalled();
  });

  it('fails closed on retry when storage read throws without rotating the prior key', async () => {
    mockClient.list.mockResolvedValue([pendingItem]);
    mockClient.approve.mockRejectedValue(new Error('ambiguous network'));
    const user = userEvent.setup();

    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.type(screen.getByTestId('action-request-reason'), 'keep key');
    await user.click(screen.getByTestId('action-request-approve'));
    await waitFor(() => expect(mockClient.approve).toHaveBeenCalledTimes(1));
    const firstKey = mockClient.approve.mock.calls[0]?.[1]?.idempotencyKey as string;
    const before = window.localStorage.getItem(physicalStorageKey());
    expect(before).toContain(firstKey);

    let writeCount = 0;
    setActionRequestIntentStorageAdapter({
      getItem() {
        throw new Error('transient storage read failure');
      },
      setItem() {
        writeCount += 1;
      },
      removeItem() {
        // no-op
      },
    });

    await user.click(screen.getByTestId('action-request-approve'));
    expect(await screen.findByTestId('action-request-error')).toHaveTextContent(
      'retry-key storage'
    );
    expect(mockClient.approve).toHaveBeenCalledTimes(1);
    expect(writeCount).toBe(0);

    setActionRequestIntentStorageAdapter(null);
    expect(window.localStorage.getItem(physicalStorageKey())).toBe(before);
  });

  it('fails closed when no authenticated active user is in scope', async () => {
    setActiveUserId(null);
    mockClient.list.mockResolvedValue([pendingItem]);
    const user = userEvent.setup();

    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.type(screen.getByTestId('action-request-reason'), 'no user');
    await user.click(screen.getByTestId('action-request-approve'));

    expect(await screen.findByTestId('action-request-error')).toHaveTextContent(
      'retry-key storage'
    );
    expect(mockClient.approve).not.toHaveBeenCalled();
  });

  it('shows a non-blocking warning when cleanup after success cannot rewrite storage', async () => {
    const approved = { ...pendingItem, approval_state: 'approved', row_version: 3 };
    mockClient.list.mockResolvedValue([pendingItem]);
    mockClient.approve.mockResolvedValueOnce(approved);
    mockClient.get.mockResolvedValueOnce(approved);
    const user = userEvent.setup();

    const backing = new Map<string, string>();
    let writeCount = 0;
    setActionRequestIntentStorageAdapter({
      getItem(key) {
        return backing.has(key) ? (backing.get(key) as string) : null;
      },
      setItem(key, value) {
        writeCount += 1;
        if (writeCount >= 2) {
          throw new Error('quota exceeded');
        }
        backing.set(key, value);
      },
      removeItem(key) {
        backing.delete(key);
      },
    });

    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.type(screen.getByTestId('action-request-reason'), 'persist warn');
    await user.click(screen.getByTestId('action-request-approve'));

    await waitFor(() => expect(mockClient.approve).toHaveBeenCalledTimes(1));
    expect(await screen.findByTestId('action-request-warning')).toHaveTextContent(
      'retry-key storage'
    );
  });

  it('invalidates a stale in-flight list after mutation without leaving refresh stuck', async () => {
    const approved = { ...pendingItem, approval_state: 'approved', row_version: 3 };
    const slowList = deferred<(typeof pendingItem)[]>();
    mockClient.list
      .mockResolvedValueOnce([pendingItem]) // initial
      .mockReturnValueOnce(slowList.promise); // refresh started before approve settles
    mockClient.approve.mockResolvedValueOnce(approved);
    mockClient.get.mockResolvedValueOnce(approved);
    const user = userEvent.setup();

    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);

    const refreshButton = screen.getByTestId('action-request-refresh');
    await user.click(refreshButton);
    await waitFor(() => expect(mockClient.list).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(refreshButton).toBeDisabled());

    await user.type(screen.getByTestId('action-request-reason'), 'stale race');
    await user.click(screen.getByTestId('action-request-approve'));
    await waitFor(() => expect(mockClient.approve).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(mockClient.get).toHaveBeenCalledWith(PENDING_ID));
    await waitFor(() =>
      expect(screen.queryByTestId(`action-request-row-${PENDING_ID}`)).not.toBeInTheDocument()
    );

    // Stale list must not reintroduce the pending row after mutation invalidation.
    slowList.resolve([pendingItem]);
    await waitFor(() => expect(refreshButton).not.toBeDisabled());
    expect(screen.queryByTestId(`action-request-row-${PENDING_ID}`)).not.toBeInTheDocument();
    expect(refreshButton).toHaveTextContent(/Refresh/i);
  });

  it('keeps terminal rows when filter is all after approve', async () => {
    const approved = { ...pendingItem, approval_state: 'approved', row_version: 3 };
    mockClient.list.mockResolvedValue([pendingItem]);
    mockClient.approve.mockResolvedValueOnce(approved);
    mockClient.get.mockResolvedValueOnce(approved);
    const user = userEvent.setup();

    render(<ActionRequestInbox />);
    await screen.findByTestId(`action-request-row-${PENDING_ID}`);
    await user.selectOptions(screen.getByTestId('action-request-filter'), 'all');
    await waitFor(() => expect(mockClient.list).toHaveBeenCalledTimes(2));

    await user.type(screen.getByTestId('action-request-reason'), 'keep visible');
    await user.click(screen.getByTestId('action-request-approve'));
    await waitFor(() => expect(mockClient.get).toHaveBeenCalledWith(PENDING_ID));
    expect(await screen.findByTestId('action-request-terminal')).toBeInTheDocument();
    expect(screen.getByTestId(`action-request-row-${PENDING_ID}`)).toBeInTheDocument();
  });
});

describe('action request idempotency helpers', () => {
  beforeEach(() => {
    window.localStorage.clear();
    setActionRequestIntentStorageAdapter(null);
    setActiveUserId(ACTIVE_USER);
  });

  afterEach(() => {
    setActionRequestIntentStorageAdapter(null);
    setActiveUserId(null);
  });

  it('binds the stored key to the complete command fingerprint without raw reason', () => {
    const scope = defaultScope();
    const first = getOrCreateIdempotencyKey(PENDING_ID, 'approve', 'same', 2, scope);
    expect(first.persisted).toBe(true);
    const second = getOrCreateIdempotencyKey(PENDING_ID, 'approve', 'same', 2, scope);
    expect(second.key).toBe(first.key);
    const rotated = getOrCreateIdempotencyKey(PENDING_ID, 'approve', 'different', 2, scope);
    expect(rotated.key).not.toBe(first.key);
    const raw = window.localStorage.getItem(physicalStorageKey());
    expect(raw).toBeTruthy();
    expect(raw).not.toContain('different');
    expect(raw).not.toContain('same');
    clearIdempotencyKey(PENDING_ID, 'approve', scope);
    expect(storedIdempotencyRecords()[`approve:${PENDING_ID}`]).toBeUndefined();
  });

  it('separates equal-length FNV-colliding reasons with SHA-256 fingerprints', () => {
    // Known 32-bit FNV-1a collision pair (len 8) reported by ac-codex review.
    const a = 'tgvipcjq';
    const b = 'tonydgba';
    expect(fingerprintReason(a)).not.toBe(fingerprintReason(b));
    expect(fingerprintReason(a)).toMatch(/^sha256:[0-9a-f]{64}$/);

    const scope = defaultScope();
    const first = getOrCreateIdempotencyKey(PENDING_ID, 'approve', a, 1, scope);
    const second = getOrCreateIdempotencyKey(PENDING_ID, 'approve', b, 1, scope);
    expect(first.persisted).toBe(true);
    expect(second.persisted).toBe(true);
    expect(second.key).not.toBe(first.key);
  });

  it('scopes intent stores by tenant under the active user namespace', () => {
    const a = getOrCreateIdempotencyKey(PENDING_ID, 'approve', 'reason', 1, {
      tenantId: 'tenant-a',
      activeUserId: ACTIVE_USER,
    });
    const b = getOrCreateIdempotencyKey(PENDING_ID, 'approve', 'reason', 1, {
      tenantId: 'tenant-b',
      activeUserId: ACTIVE_USER,
    });
    expect(a.key).not.toBe(b.key);
    expect(window.localStorage.getItem(physicalStorageKey('tenant-a'))).toContain(a.key);
    expect(window.localStorage.getItem(physicalStorageKey('tenant-b'))).toContain(b.key);
  });

  it('does not share intent storage across active users', () => {
    const first = getOrCreateIdempotencyKey(PENDING_ID, 'approve', 'reason', 1, defaultScope());
    expect(first.persisted).toBe(true);

    setActiveUserId('other-user');
    const second = getOrCreateIdempotencyKey(PENDING_ID, 'approve', 'reason', 1, {
      tenantId: TENANT_ID,
      activeUserId: 'other-user',
    });
    expect(second.persisted).toBe(true);
    expect(second.key).not.toBe(first.key);
    expect(window.localStorage.getItem(physicalStorageKey(TENANT_ID, ACTIVE_USER))).toContain(
      first.key
    );
    expect(window.localStorage.getItem(physicalStorageKey(TENANT_ID, 'other-user'))).toContain(
      second.key
    );
  });

  it('returns persisted=false when the initial write fails', () => {
    setActionRequestIntentStorageAdapter({
      getItem: () => null,
      setItem: () => {
        throw new Error('boom');
      },
      removeItem: () => undefined,
    });
    const result = getOrCreateIdempotencyKey(PENDING_ID, 'approve', 'x', 1, defaultScope());
    expect(result.persisted).toBe(false);
  });

  it('does not rotate an existing retry key when storage read throws', () => {
    const scope = defaultScope();
    const first = getOrCreateIdempotencyKey(PENDING_ID, 'approve', 'same intent', 2, scope);
    expect(first.persisted).toBe(true);
    const before = window.localStorage.getItem(physicalStorageKey());
    expect(before).toContain(first.key);

    let writeCount = 0;
    setActionRequestIntentStorageAdapter({
      getItem() {
        throw new Error('transient storage read failure');
      },
      setItem() {
        writeCount += 1;
      },
      removeItem() {
        // no-op
      },
    });

    const retry = getOrCreateIdempotencyKey(PENDING_ID, 'approve', 'same intent', 2, scope);
    expect(retry.persisted).toBe(false);
    expect(retry.key).toBe('');
    expect(writeCount).toBe(0);

    // Prior durable intent must remain untouched under the real user-scoped path.
    setActionRequestIntentStorageAdapter(null);
    expect(window.localStorage.getItem(physicalStorageKey())).toBe(before);
    const recovered = getOrCreateIdempotencyKey(PENDING_ID, 'approve', 'same intent', 2, scope);
    expect(recovered.persisted).toBe(true);
    expect(recovered.key).toBe(first.key);
  });

  it('returns persisted=false without an active user scope', () => {
    setActiveUserId(null);
    expect(resolveActiveUserScope()).toBeNull();
    const result = getOrCreateIdempotencyKey(PENDING_ID, 'approve', 'x', 1, {
      tenantId: TENANT_ID,
      activeUserId: '',
    });
    expect(result.persisted).toBe(false);
  });

  it('clears both approve and reject keys on terminal cleanup', () => {
    const scope = defaultScope();
    getOrCreateIdempotencyKey(PENDING_ID, 'approve', 'a', 1, scope);
    getOrCreateIdempotencyKey(PENDING_ID, 'reject', 'b', 1, scope);
    expect(storedIdempotencyRecords()[`approve:${PENDING_ID}`]).toBeDefined();
    expect(storedIdempotencyRecords()[`reject:${PENDING_ID}`]).toBeDefined();
    expect(clearAllDecisionIdempotencyKeys(PENDING_ID, scope)).toBe(true);
    expect(storedIdempotencyRecords()[`approve:${PENDING_ID}`]).toBeUndefined();
    expect(storedIdempotencyRecords()[`reject:${PENDING_ID}`]).toBeUndefined();
  });

  it('accepts an injected adapter for deterministic storage tests', () => {
    const adapter = memoryStorageAdapter();
    setActionRequestIntentStorageAdapter(adapter);
    const scope = { tenantId: 't', activeUserId: ACTIVE_USER };
    const first = getOrCreateIdempotencyKey(PENDING_ID, 'approve', 'memo', 9, scope);
    expect(first.persisted).toBe(true);
    const second = getOrCreateIdempotencyKey(PENDING_ID, 'approve', 'memo', 9, scope);
    expect(second.key).toBe(first.key);
  });
});
