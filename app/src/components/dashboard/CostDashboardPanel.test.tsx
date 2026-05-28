import { configureStore } from '@reduxjs/toolkit';
import { render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../services/coreRpcClient';
import CostDashboardPanel from './CostDashboardPanel';

vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

vi.mock('recharts', async () => {
  // Recharts uses ResponsiveContainer which needs DOM measurements that
  // jsdom does not provide. Stub the few primitives the dashboard pulls in
  // with passthrough divs so we can assert on the panel structure without
  // pulling the real chart pipeline into the test runtime.
  const passthrough = ({ children }: { children?: React.ReactNode }) => (
    <div data-stub="recharts">{children}</div>
  );
  const stub = () => null;
  return {
    Bar: passthrough,
    BarChart: passthrough,
    Cell: stub,
    LabelList: stub,
    Legend: stub,
    ReferenceLine: stub,
    ResponsiveContainer: passthrough,
    Tooltip: stub,
    XAxis: stub,
    YAxis: stub,
  };
});

const mockedCall = callCoreRpc as unknown as ReturnType<typeof vi.fn>;

function makeStore() {
  return configureStore({ reducer: { locale: (state = { current: 'en' }) => state } });
}

function renderPanel() {
  return render(
    <Provider store={makeStore()}>
      <MemoryRouter initialEntries={['/settings/cost-dashboard']}>
        <CostDashboardPanel />
      </MemoryRouter>
    </Provider>
  );
}

describe('<CostDashboardPanel />', () => {
  beforeEach(() => {
    mockedCall.mockReset();
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it('shows the loading state and then renders all sections', async () => {
    mockedCall.mockResolvedValueOnce({
      days: Array.from({ length: 7 }, (_, i) => ({
        date: `2026-05-${21 + i}`,
        cost_usd: i === 6 ? 1.25 : 0,
        input_tokens: i === 6 ? 1000 : 0,
        output_tokens: i === 6 ? 500 : 0,
        total_tokens: i === 6 ? 1500 : 0,
        request_count: i === 6 ? 1 : 0,
        by_model: [],
      })),
      period_total_usd: 1.25,
      monthly_pace_usd: 5.36,
      budget_limit_monthly_usd: 100,
      month_to_date_usd: 1.25,
      budget_utilization: 0.0125,
      budget_status: 'normal',
      currency: 'USD',
      warn_threshold: 0.8,
      alert_threshold: 0.95,
      enabled: true,
      by_model: [
        {
          model: 'anthropic/claude-sonnet-4',
          cost_usd: 1.25,
          total_tokens: 1500,
          request_count: 1,
          provider: 'anthropic',
          percent_of_total: 100,
        },
      ],
    });
    renderPanel();
    await waitFor(() => expect(screen.getByTestId('cost-dashboard-summary')).toBeInTheDocument());
    expect(screen.getByTestId('cost-dashboard-cost-chart')).toBeInTheDocument();
    expect(screen.getByTestId('cost-dashboard-token-chart')).toBeInTheDocument();
    expect(screen.getByTestId('cost-dashboard-model-table')).toBeInTheDocument();
  });

  it('shows the disabled hint when the payload reports enabled=false', async () => {
    mockedCall.mockResolvedValueOnce({
      days: [],
      period_total_usd: 0,
      monthly_pace_usd: 0,
      budget_limit_monthly_usd: 0,
      month_to_date_usd: 0,
      budget_utilization: 0,
      budget_status: 'normal',
      currency: 'USD',
      warn_threshold: 0.8,
      alert_threshold: 0.95,
      enabled: false,
      by_model: [],
    });
    renderPanel();
    await waitFor(() => expect(screen.getByTestId('cost-dashboard-disabled')).toBeInTheDocument());
  });

  it('surfaces the error state when the RPC rejects', async () => {
    mockedCall.mockRejectedValueOnce(new Error('rpc down'));
    renderPanel();
    await waitFor(() =>
      expect(screen.getByTestId('cost-dashboard-error')).toHaveTextContent(/rpc down/)
    );
  });
});
