import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import DashboardSkeleton from './DashboardSkeleton';

describe('<DashboardSkeleton />', () => {
  it('renders the loading skeleton container with accessible status role', () => {
    render(<DashboardSkeleton />);
    const node = screen.getByTestId('cost-dashboard-skeleton');
    expect(node).toBeInTheDocument();
    expect(node).toHaveAttribute('role', 'status');
    expect(node).toHaveAttribute('aria-live', 'polite');
  });
});
