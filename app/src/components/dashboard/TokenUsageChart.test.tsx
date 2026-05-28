import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { CostDashboardDay } from '../../hooks/useCostDashboard';
import TokenUsageChart from './TokenUsageChart';

vi.mock('recharts', () => {
  // Stub recharts primitives — jsdom lacks the layout measurements
  // recharts' ResponsiveContainer needs. Passthrough divs are enough
  // for the smoke test below; we only assert the wrapper renders.
  const passthrough = ({ children }: { children?: React.ReactNode }) => (
    <div data-stub="recharts">{children}</div>
  );
  const stub = () => null;
  return {
    Bar: stub,
    BarChart: passthrough,
    Legend: stub,
    ResponsiveContainer: passthrough,
    Tooltip: stub,
    XAxis: stub,
    YAxis: stub,
  };
});

const days: CostDashboardDay[] = Array.from({ length: 7 }, (_, i) => ({
  date: `2026-05-${21 + i}`,
  cost_usd: i === 6 ? 1.25 : 0,
  input_tokens: i === 6 ? 1000 : 0,
  output_tokens: i === 6 ? 500 : 0,
  total_tokens: i === 6 ? 1500 : 0,
  request_count: i === 6 ? 1 : 0,
  by_model: [],
}));

describe('<TokenUsageChart />', () => {
  it('renders the chart wrapper with the expected test-id', () => {
    render(<TokenUsageChart days={days} />);
    expect(screen.getByTestId('token-usage-chart')).toBeInTheDocument();
  });

  it('renders gracefully when given an empty day list', () => {
    render(<TokenUsageChart days={[]} />);
    expect(screen.getByTestId('token-usage-chart')).toBeInTheDocument();
  });
});
