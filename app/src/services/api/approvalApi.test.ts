import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  type ApprovalAuditEntry,
  fetchPendingApprovals,
  fetchRecentApprovalDecisions,
  unwrapRows,
} from './approvalApi';

const mockCallCoreRpc = vi.fn();

vi.mock('../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

const auditRow = (overrides: Partial<ApprovalAuditEntry> = {}): ApprovalAuditEntry => ({
  request_id: 'req-1',
  tool_name: 'shell',
  action_summary: 'run ls',
  args_redacted: {},
  session_id: 'sess-1',
  created_at: '2026-05-29T10:00:00Z',
  expires_at: null,
  decided_at: '2026-05-29T10:00:05Z',
  decision: 'approve_once',
  ...overrides,
});

describe('unwrapRows', () => {
  it('returns a bare array as-is (gate absent path)', () => {
    expect(unwrapRows([1, 2, 3])).toEqual([1, 2, 3]);
  });

  it('unwraps the {result, logs} envelope (gate installed path)', () => {
    expect(unwrapRows({ result: [{ a: 1 }], logs: ['note'] })).toEqual([{ a: 1 }]);
  });

  it('returns [] for null / non-array / malformed shapes rather than throwing', () => {
    expect(unwrapRows(null)).toEqual([]);
    expect(unwrapRows(undefined)).toEqual([]);
    expect(unwrapRows({ result: 'nope' })).toEqual([]);
    expect(unwrapRows(42)).toEqual([]);
  });
});

describe('fetchRecentApprovalDecisions', () => {
  beforeEach(() => mockCallCoreRpc.mockReset());

  it('calls the correct method with no params when limit omitted', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: [auditRow()], logs: ['x'] });

    const rows = await fetchRecentApprovalDecisions();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.approval_list_recent_decisions',
      params: {},
    });
    expect(rows).toHaveLength(1);
    expect(rows[0].decision).toBe('approve_once');
  });

  it('forwards an explicit limit', async () => {
    mockCallCoreRpc.mockResolvedValueOnce([]);

    await fetchRecentApprovalDecisions(10);

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.approval_list_recent_decisions',
      params: { limit: 10 },
    });
  });

  it('normalizes a bare-array response (gate absent)', async () => {
    mockCallCoreRpc.mockResolvedValueOnce([]);
    expect(await fetchRecentApprovalDecisions()).toEqual([]);
  });
});

describe('fetchPendingApprovals', () => {
  beforeEach(() => mockCallCoreRpc.mockReset());

  it('calls the pending method and unwraps the envelope', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      result: [{ request_id: 'p-1', tool_name: 'curl' }],
      logs: ['1 row'],
    });

    const rows = await fetchPendingApprovals();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.approval_list_pending' });
    expect(rows[0].request_id).toBe('p-1');
  });
});
