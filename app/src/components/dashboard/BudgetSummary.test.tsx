import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import BudgetSummary from './BudgetSummary';

describe('<BudgetSummary />', () => {
  it('renders all four metric tiles and the status badge', () => {
    render(
      <BudgetSummary
        currency="USD"
        periodTotalUsd={42.5}
        monthlyPaceUsd={181.25}
        budgetLimitMonthlyUsd={200}
        monthToDateUsd={50}
        utilization={0.25}
        status="normal"
      />
    );
    expect(screen.getByTestId('metric-total-spend')).toBeInTheDocument();
    expect(screen.getByTestId('metric-monthly-pace')).toBeInTheDocument();
    expect(screen.getByTestId('metric-budget-limit')).toBeInTheDocument();
    expect(screen.getByTestId('utilization-fill')).toBeInTheDocument();
    expect(screen.getByTestId('budget-status-badge')).toHaveTextContent(/on track/i);
  });

  it('reflects "warning" status on the badge', () => {
    render(
      <BudgetSummary
        currency="USD"
        periodTotalUsd={1}
        monthlyPaceUsd={1}
        budgetLimitMonthlyUsd={10}
        monthToDateUsd={8}
        utilization={0.85}
        status="warning"
      />
    );
    expect(screen.getByTestId('budget-status-badge')).toHaveTextContent(/warning/i);
  });

  it('reflects "exceeded" status and clamps utilisation bar to 100%', () => {
    render(
      <BudgetSummary
        currency="USD"
        periodTotalUsd={1}
        monthlyPaceUsd={1}
        budgetLimitMonthlyUsd={10}
        monthToDateUsd={15}
        utilization={1}
        status="exceeded"
      />
    );
    expect(screen.getByTestId('budget-status-badge')).toHaveTextContent(/over budget/i);
    expect(screen.getByTestId('utilization-fill')).toHaveStyle({ width: '100%' });
  });

  it('falls back to "No limit set" when budget is zero', () => {
    render(
      <BudgetSummary
        currency="USD"
        periodTotalUsd={5}
        monthlyPaceUsd={5}
        budgetLimitMonthlyUsd={0}
        monthToDateUsd={0}
        utilization={0}
        status="normal"
      />
    );
    expect(screen.getByTestId('metric-budget-limit')).toHaveTextContent(/no limit set/i);
  });
});
