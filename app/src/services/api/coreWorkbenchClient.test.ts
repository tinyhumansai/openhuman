import { beforeEach, describe, expect, it, vi } from 'vitest';

import { CORE_RPC_METHODS } from '../rpcMethods';
import { createCoreWorkbenchClient } from './coreWorkbenchClient';

const mockCallCoreRpc = vi.fn();

vi.mock('../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

const alert = (overrides = {}) => ({
  id: 'alert-1',
  alert_type: 'missed_checkin',
  severity: 'critical',
  related_type: 'task_instance',
  related_id: 'task-1',
  status: 'open',
  summary: 'Owner missed check-in.',
  created_at: '2026-06-01T00:00:00Z',
  context: null,
  ...overrides,
});

const trace = (overrides = {}) => ({
  alert_id: 'alert-1',
  workflow: {
    type: 'health_plan',
    id: 'plan-1',
    task_id: 'task-1',
    openclaw_flow_id: 'flow-plan-1',
  },
  partial: true,
  warnings: [
    { code: 'trace_truncated', message: 'Trace limited to 50 entries', source: 'event_outbox' },
  ],
  entries: [
    {
      id: 'event:event-1',
      occurred_at: '2026-06-01T00:00:00Z',
      kind: 'outbox_event',
      source: 'event_outbox',
      title: 'Event emitted',
      detail: null,
      actor: null,
      related_type: null,
      related_id: null,
      severity: null,
      metadata: { event_type: 'task.checkin_received', nested: { b: 2 } },
    },
  ],
  ...overrides,
});

describe('coreWorkbenchClient', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('lists Core alerts through the core RPC bridge with filters', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: [alert()], logs: ['listed'] });
    const client = createCoreWorkbenchClient({ timeoutMs: 12_000 });

    const alerts = await client.listAlerts({ status: 'open', severity: 'critical' });

    expect(alerts).toHaveLength(1);
    expect(alerts[0]?.id).toBe('alert-1');
    expect(alerts[0]?.context).toBeNull();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: CORE_RPC_METHODS.youpetListAlerts,
      params: { status: 'open', severity: 'critical' },
      timeoutMs: 12_000,
    });
  });

  it('preserves operational context from listed Core alerts', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      result: [
        alert({
          context: {
            pet: {
              id: 'pet-1',
              name: 'Mochi',
              species: 'cat',
              breed: 'Exotic Shorthair',
              status: 'active',
            },
            owner: { id: 'owner-1', name: 'Owner A', phone: '18800000001', status: 'active' },
            health_plan: {
              id: 'plan-1',
              title: 'Daily check-in',
              plan_type: 'checkin',
              status: 'active',
              openclaw_flow_id: 'flow-plan-1',
            },
            task: {
              id: 'task-1',
              status: 'missed',
              due_at: '2026-06-01T10:01:00Z',
              missed_count: 2,
              openclaw_flow_id: 'flow-task-1',
            },
            latest_checkin: {
              id: 'checkin-1',
              submitted_at: '2026-06-01T10:10:00Z',
              submitted_by: 'owner-1',
              text: 'Looks normal.',
              status_tags: ['normal'],
            },
          },
        }),
      ],
      logs: ['listed'],
    });
    const client = createCoreWorkbenchClient();

    const alerts = await client.listAlerts();

    expect(alerts[0]?.context?.pet.name).toBe('Mochi');
    expect(alerts[0]?.context?.health_plan.openclaw_flow_id).toBe('flow-plan-1');
    expect(alerts[0]?.context?.latest_checkin?.status_tags).toEqual(['normal']);
  });

  it('passes null status through so Rust can request all alert states', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: [], logs: [] });
    const client = createCoreWorkbenchClient();

    await client.listAlerts({ status: null });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: CORE_RPC_METHODS.youpetListAlerts,
      params: { status: null },
      timeoutMs: undefined,
    });
  });

  it('acknowledges alerts through the core RPC bridge and forwards caller key', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({
      result: alert({ status: 'acknowledged' }),
      logs: ['acknowledged'],
    });
    const client = createCoreWorkbenchClient();

    const updated = await client.ackAlert('alert-1', {
      note: 'Calling owner.',
      idempotencyKey: 'idem-ack-1',
    });

    expect(updated.status).toBe('acknowledged');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: CORE_RPC_METHODS.youpetAckAlert,
      params: { alertId: 'alert-1', note: 'Calling owner.', idempotencyKey: 'idem-ack-1' },
      timeoutMs: undefined,
    });
  });

  it('accepts action results that omit list-only context', async () => {
    const { context: _context, ...actionResult } = alert({ status: 'acknowledged' });
    mockCallCoreRpc.mockResolvedValueOnce({ result: actionResult });
    const client = createCoreWorkbenchClient();

    const updated = await client.ackAlert('alert-1', { idempotencyKey: 'idem-ack-2' });

    expect(updated.status).toBe('acknowledged');
    expect(updated.context).toBeUndefined();
  });

  it('resolves alerts through the core RPC bridge', async () => {
    mockCallCoreRpc.mockResolvedValueOnce(alert({ status: 'resolved' }));
    const client = createCoreWorkbenchClient();

    const updated = await client.resolveAlert('alert-1', {
      resolution: 'Owner confirmed completion.',
    });

    expect(updated.status).toBe('resolved');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: CORE_RPC_METHODS.youpetResolveAlert,
      params: {
        alertId: 'alert-1',
        resolution: 'Owner confirmed completion.',
        idempotencyKey: undefined,
      },
      timeoutMs: undefined,
    });
  });

  it('loads alert traces through the core RPC bridge', async () => {
    mockCallCoreRpc.mockResolvedValueOnce({ result: trace(), logs: ['traced'] });
    const client = createCoreWorkbenchClient({ timeoutMs: 8_000 });

    const loaded = await client.getAlertTrace('alert-1');

    expect(loaded.partial).toBe(true);
    expect(loaded.workflow?.id).toBe('plan-1');
    expect(loaded.warnings[0]?.code).toBe('trace_truncated');
    expect(loaded.entries[0]?.kind).toBe('outbox_event');
    expect(loaded.entries[0]?.metadata).toEqual({
      event_type: 'task.checkin_received',
      nested: { b: 2 },
    });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: CORE_RPC_METHODS.youpetTraceAlert,
      params: { alertId: 'alert-1' },
      timeoutMs: 8_000,
    });
  });

  it('unwraps raw alert trace responses too', async () => {
    mockCallCoreRpc.mockResolvedValueOnce(trace({ partial: false, warnings: [] }));
    const client = createCoreWorkbenchClient();

    const loaded = await client.getAlertTrace('alert-1');

    expect(loaded.partial).toBe(false);
    expect(loaded.entries[0]?.source).toBe('event_outbox');
  });

  it('preserves future trace literal strings from Core', async () => {
    mockCallCoreRpc.mockResolvedValueOnce(
      trace({
        warnings: [{ code: 'future_warning', message: 'Future warning', source: 'future_source' }],
        entries: [
          {
            ...trace().entries[0],
            kind: 'future_kind',
            source: 'future_source',
            severity: 'future_severity',
          },
        ],
      })
    );
    const client = createCoreWorkbenchClient();

    const loaded = await client.getAlertTrace('alert-1');

    expect(loaded.warnings[0]?.code).toBe('future_warning');
    expect(loaded.warnings[0]?.source).toBe('future_source');
    expect(loaded.entries[0]?.kind).toBe('future_kind');
    expect(loaded.entries[0]?.source).toBe('future_source');
    expect(loaded.entries[0]?.severity).toBe('future_severity');
  });

  it('accepts ActionRequest trace literals from Core unchanged', async () => {
    mockCallCoreRpc.mockResolvedValueOnce(
      trace({
        warnings: [
          {
            code: 'action_request_deliveries_truncated',
            message: 'ActionRequest delivery rows truncated',
            source: 'outbox_deliveries',
          },
          {
            code: 'invalid_action_request_projection',
            message: 'ActionRequest document could not be projected',
            source: 'action_requests',
          },
        ],
        entries: [
          {
            ...trace().entries[0],
            kind: 'action_request_proposed',
            source: 'action_requests',
            title: 'ActionRequest proposed',
          },
          {
            ...trace().entries[0],
            id: 'action-request:req-1:approved',
            kind: 'action_request_approved',
            source: 'action_requests',
            title: 'ActionRequest approved',
          },
          {
            ...trace().entries[0],
            id: 'action-request:req-2:rejected',
            kind: 'action_request_rejected',
            source: 'action_requests',
            title: 'ActionRequest rejected',
          },
          {
            ...trace().entries[0],
            id: 'action-request:req-1:execution',
            kind: 'action_request_execution',
            source: 'action_requests',
            title: 'ActionRequest execution succeeded',
          },
        ],
      })
    );
    const client = createCoreWorkbenchClient();

    const loaded = await client.getAlertTrace('alert-1');

    expect(loaded.warnings[0]?.code).toBe('action_request_deliveries_truncated');
    expect(loaded.warnings[0]?.source).toBe('outbox_deliveries');
    expect(loaded.warnings[1]?.code).toBe('invalid_action_request_projection');
    expect(loaded.warnings[1]?.source).toBe('action_requests');
    expect(loaded.entries.map(entry => entry.kind)).toEqual([
      'action_request_proposed',
      'action_request_approved',
      'action_request_rejected',
      'action_request_execution',
    ]);
    expect(loaded.entries.every(entry => entry.source === 'action_requests')).toBe(true);
  });

  it('propagates RPC errors to callers', async () => {
    mockCallCoreRpc.mockRejectedValueOnce(new Error('invalid_task_state'));
    const client = createCoreWorkbenchClient();

    await expect(client.ackAlert('alert-1', {})).rejects.toThrow('invalid_task_state');
  });
});
