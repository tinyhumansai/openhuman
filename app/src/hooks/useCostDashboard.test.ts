import { renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../services/coreRpcClient';
import { type CostDashboardPayload, useCostDashboard } from './useCostDashboard';

vi.mock('../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const mockedCall = callCoreRpc as unknown as ReturnType<typeof vi.fn>;

const fixture: CostDashboardPayload = {
  days: [
    {
      date: '2026-05-21',
      cost_usd: 0,
      input_tokens: 0,
      output_tokens: 0,
      total_tokens: 0,
      request_count: 0,
      by_model: [],
    },
  ],
  period_total_usd: 0,
  monthly_pace_usd: 0,
  budget_limit_monthly_usd: 100,
  month_to_date_usd: 0,
  budget_utilization: 0,
  budget_status: 'normal',
  currency: 'USD',
  warn_threshold: 0.8,
  alert_threshold: 0.95,
  enabled: true,
  by_model: [],
};

describe('useCostDashboard', () => {
  beforeEach(() => {
    mockedCall.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('calls openhuman.cost_get_dashboard and surfaces the payload', async () => {
    mockedCall.mockResolvedValueOnce(fixture);
    const { result } = renderHook(() => useCostDashboard({ paused: true }));
    await waitFor(() => expect(result.current.data).not.toBeNull());
    expect(mockedCall).toHaveBeenCalledWith(
      expect.objectContaining({ method: 'openhuman.cost_get_dashboard' })
    );
    expect(result.current.error).toBeNull();
    expect(result.current.data?.currency).toBe('USD');
  });

  it('unwraps RpcOutcome `{ result, logs }` envelopes', async () => {
    mockedCall.mockResolvedValueOnce({ result: fixture, logs: ['info'] });
    const { result } = renderHook(() => useCostDashboard({ paused: true }));
    await waitFor(() => expect(result.current.data).not.toBeNull());
    expect(result.current.data?.currency).toBe('USD');
  });

  it('reports error state when the RPC rejects', async () => {
    mockedCall.mockRejectedValueOnce(new Error('boom'));
    const { result } = renderHook(() => useCostDashboard({ paused: true }));
    await waitFor(() => expect(result.current.error).toBe('boom'));
    expect(result.current.data).toBeNull();
  });
});
