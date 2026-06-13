import { useCallback, useEffect, useRef, useState } from 'react';

import { callCoreRpc } from '../services/coreRpcClient';

export type BudgetStatus = 'normal' | 'warning' | 'exceeded';

export interface CostDashboardModelStats {
  model: string;
  cost_usd: number;
  total_tokens: number;
  request_count: number;
  provider: string | null;
  percent_of_total: number;
}

export interface CostDashboardDay {
  date: string;
  cost_usd: number;
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  request_count: number;
  by_model: CostDashboardModelStats[];
}

export interface CostDashboardPayload {
  days: CostDashboardDay[];
  period_total_usd: number;
  monthly_pace_usd: number;
  budget_limit_monthly_usd: number;
  month_to_date_usd: number;
  budget_utilization: number;
  budget_status: BudgetStatus;
  currency: string;
  warn_threshold: number;
  alert_threshold: number;
  enabled: boolean;
  by_model: CostDashboardModelStats[];
}

export interface CostUsageRecord {
  id: string;
  timestamp: string;
  session_id: string;
  model: string;
  provider: string | null;
  category: string;
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  cost_usd: number;
}

export interface CostUsageCategoryStats {
  category: string;
  cost_usd: number;
  total_tokens: number;
  request_count: number;
  percent_of_total: number;
}

export interface CostUsageLogPayload {
  records: CostUsageRecord[];
  by_category: CostUsageCategoryStats[];
  total_cost_usd: number;
  total_tokens: number;
  request_count: number;
  currency: string;
  days: number;
  limit: number;
}

interface RpcEnvelope<T> {
  result?: T;
  logs?: string[];
}

const DEFAULT_REFRESH_MS = 10_000;

export interface UseCostDashboardOptions {
  refreshMs?: number;
  /** When true, polling pauses (e.g. when the panel is hidden). */
  paused?: boolean;
}

export interface UseCostDashboardResult {
  data: CostDashboardPayload | null;
  /** True only before the first successful fetch resolves. */
  isLoading: boolean;
  /** True whenever a background refresh is in flight (initial load or poll). */
  isFetching: boolean;
  error: string | null;
  /** Wall-clock ms when `data` was last successfully populated, or `null`. */
  lastUpdated: number | null;
  /** Manual refetch — does not toggle `isLoading` if `data` is already present. */
  refetch: () => Promise<void>;
}

export interface UseCostUsageLogOptions extends UseCostDashboardOptions {
  days?: number;
  limit?: number;
}

export interface UseCostUsageLogResult {
  data: CostUsageLogPayload | null;
  isLoading: boolean;
  isFetching: boolean;
  error: string | null;
  lastUpdated: number | null;
  refetch: () => Promise<void>;
}

function unwrapRpcPayload<T>(response: RpcEnvelope<T> | T): T {
  return response && typeof response === 'object' && 'result' in response && response.result
    ? (response.result as T)
    : (response as T);
}

/**
 * Fetches the 7-day cost dashboard payload from the core via JSON-RPC and
 * polls every `refreshMs` (default 10s) so today's bar and summary metrics
 * stay live without a page refresh.
 */
export function useCostDashboard(options: UseCostDashboardOptions = {}): UseCostDashboardResult {
  const { refreshMs = DEFAULT_REFRESH_MS, paused = false } = options;
  const [data, setData] = useState<CostDashboardPayload | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState<boolean>(true);
  const [isFetching, setIsFetching] = useState<boolean>(true);
  const [lastUpdated, setLastUpdated] = useState<number | null>(null);
  const cancelledRef = useRef<boolean>(false);

  const fetchOnce = useCallback(async () => {
    setIsFetching(true);
    try {
      const response = await callCoreRpc<RpcEnvelope<CostDashboardPayload> | CostDashboardPayload>({
        method: 'openhuman.cost_get_dashboard',
        params: {},
      });
      if (cancelledRef.current) return;
      const payload = unwrapRpcPayload(response);
      setData(payload);
      setError(null);
      setLastUpdated(Date.now());
    } catch (err) {
      if (cancelledRef.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (!cancelledRef.current) {
        setIsLoading(false);
        setIsFetching(false);
      }
    }
  }, []);

  const refetch = useCallback(async () => {
    await fetchOnce();
  }, [fetchOnce]);

  useEffect(() => {
    cancelledRef.current = false;
    // Always fire one fetch on mount so the panel has data to render even
    // when polling is paused (background tab, hidden panel). `paused`
    // only suppresses the periodic interval — not the initial load —
    // so the user never sees a blank chart on first navigation. If you
    // need a fully-inert hook, gate the call site on the same flag.
    void fetchOnce();
    if (paused) {
      return () => {
        cancelledRef.current = true;
      };
    }
    const interval = window.setInterval(
      () => {
        void fetchOnce();
      },
      Math.max(1000, refreshMs)
    );
    return () => {
      cancelledRef.current = true;
      window.clearInterval(interval);
    };
  }, [fetchOnce, refreshMs, paused]);

  return { data, isLoading, isFetching, error, lastUpdated, refetch };
}

/**
 * Fetches detailed persisted cost rows and inferred category distribution.
 */
export function useCostUsageLog(options: UseCostUsageLogOptions = {}): UseCostUsageLogResult {
  const { refreshMs = DEFAULT_REFRESH_MS, paused = false, days = 30, limit = 250 } = options;
  const [data, setData] = useState<CostUsageLogPayload | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState<boolean>(true);
  const [isFetching, setIsFetching] = useState<boolean>(true);
  const [lastUpdated, setLastUpdated] = useState<number | null>(null);
  const cancelledRef = useRef<boolean>(false);

  const fetchOnce = useCallback(async () => {
    setIsFetching(true);
    try {
      const response = await callCoreRpc<RpcEnvelope<CostUsageLogPayload> | CostUsageLogPayload>({
        method: 'openhuman.cost_get_usage_log',
        params: { days, limit },
      });
      if (cancelledRef.current) return;
      setData(unwrapRpcPayload(response));
      setError(null);
      setLastUpdated(Date.now());
    } catch (err) {
      if (cancelledRef.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (!cancelledRef.current) {
        setIsLoading(false);
        setIsFetching(false);
      }
    }
  }, [days, limit]);

  const refetch = useCallback(async () => {
    await fetchOnce();
  }, [fetchOnce]);

  useEffect(() => {
    cancelledRef.current = false;
    void fetchOnce();
    if (paused) {
      return () => {
        cancelledRef.current = true;
      };
    }
    const interval = window.setInterval(
      () => {
        void fetchOnce();
      },
      Math.max(1000, refreshMs)
    );
    return () => {
      cancelledRef.current = true;
      window.clearInterval(interval);
    };
  }, [fetchOnce, refreshMs, paused]);

  return { data, isLoading, isFetching, error, lastUpdated, refetch };
}
