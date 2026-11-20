import { act, renderHook, waitFor } from '@testing-library/react';
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

  it.each(['success', 'failure'] as const)(
    'lets a slow request settle with %s before starting another scheduled poll',
    async outcome => {
      vi.useFakeTimers();
      let settle!: () => void;
      mockedCall
        .mockImplementationOnce(
          () =>
            new Promise<CostDashboardPayload>((resolve, reject) => {
              settle = () =>
                outcome === 'success' ? resolve(fixture) : reject(new Error('Slow RPC failed'));
            })
        )
        .mockResolvedValue(fixture);
      const { result } = renderHook(() => useCostDashboard());

      await act(async () => {
        await vi.advanceTimersByTimeAsync(12_000);
      });
      expect(mockedCall).toHaveBeenCalledTimes(1);
      await act(async () => {
        settle();
      });
      expect(result.current.isLoading).toBe(false);
      expect(result.current.isFetching).toBe(false);
      expect(result.current.error).toBe(outcome === 'failure' ? 'Slow RPC failed' : null);
      expect(result.current.data).toEqual(outcome === 'success' ? fixture : null);

      await act(async () => {
        await vi.advanceTimersByTimeAsync(8_000);
      });
      expect(mockedCall).toHaveBeenCalledTimes(2);
      expect(result.current.data).toEqual(fixture);
      expect(result.current.error).toBeNull();
    }
  );

  it('keeps the newest refresh when an earlier dashboard request finishes later', async () => {
    let resolvePrevious!: (value: CostDashboardPayload) => void;
    mockedCall
      .mockImplementationOnce(
        () =>
          new Promise<CostDashboardPayload>(resolve => {
            resolvePrevious = resolve;
          })
      )
      .mockResolvedValueOnce({ ...fixture, period_total_usd: 20 });
    const { result } = renderHook(() => useCostDashboard({ paused: true }));

    await act(async () => {
      await result.current.refetch();
    });
    expect(result.current.data?.period_total_usd).toBe(20);
    await act(async () => {
      resolvePrevious({ ...fixture, period_total_usd: 10 });
    });

    expect(result.current.data?.period_total_usd).toBe(20);
    expect(result.current.error).toBeNull();
  });

  it('ignores an earlier request failure after a newer refresh succeeds', async () => {
    let rejectPrevious!: (error: Error) => void;
    mockedCall
      .mockImplementationOnce(
        () =>
          new Promise<CostDashboardPayload>((_resolve, reject) => {
            rejectPrevious = reject;
          })
      )
      .mockResolvedValueOnce(fixture);
    const { result } = renderHook(() => useCostDashboard({ paused: true }));

    await act(async () => {
      await result.current.refetch();
    });
    await act(async () => {
      rejectPrevious(new Error('Stale request failed'));
    });

    expect(result.current.data).toEqual(fixture);
    expect(result.current.error).toBeNull();
    expect(result.current.isFetching).toBe(false);
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

  it('unwraps Outcome `{ result, logs }` envelopes', async () => {
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
