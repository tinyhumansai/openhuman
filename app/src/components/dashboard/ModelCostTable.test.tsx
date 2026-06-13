import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { CostDashboardModelStats } from '../../hooks/useCostDashboard';
import ModelCostTable from './ModelCostTable';

const sample: CostDashboardModelStats[] = [
  {
    model: 'anthropic/claude-sonnet-4',
    cost_usd: 2.5,
    total_tokens: 12000,
    request_count: 4,
    provider: 'anthropic',
    percent_of_total: 50.0,
  },
  {
    model: 'openai/gpt-5',
    cost_usd: 1.0,
    total_tokens: 4000,
    request_count: 2,
    provider: 'openai',
    percent_of_total: 20.0,
  },
];

describe('<ModelCostTable />', () => {
  it('renders one row per model with cost, tokens, requests, and percent', () => {
    render(<ModelCostTable models={sample} currency="USD" />);
    expect(screen.getByTestId('model-row-anthropic/claude-sonnet-4')).toBeInTheDocument();
    expect(screen.getByTestId('model-row-openai/gpt-5')).toBeInTheDocument();
    expect(screen.getByText(/50\.0%/)).toBeInTheDocument();
    expect(screen.getByText(/20\.0%/)).toBeInTheDocument();
    // Model name suffix renders in the row (full id available via title attr).
    expect(screen.getByText(/claude-sonnet-4/)).toBeInTheDocument();
    expect(screen.getByTitle('anthropic/claude-sonnet-4')).toBeInTheDocument();
  });

  it('renders an empty-state row when no models are present', () => {
    render(<ModelCostTable models={[]} currency="USD" />);
    expect(screen.getByTestId('model-cost-table-empty')).toBeInTheDocument();
  });

  it('renders a dash when the provider is unknown', () => {
    render(
      <ModelCostTable
        models={[
          {
            model: 'rogue-model',
            cost_usd: 0.1,
            total_tokens: 100,
            request_count: 1,
            provider: null,
            percent_of_total: 1,
          },
        ]}
        currency="USD"
      />
    );
    expect(screen.getByText('—')).toBeInTheDocument();
  });
});
