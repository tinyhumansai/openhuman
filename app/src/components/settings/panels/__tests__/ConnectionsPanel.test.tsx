import { screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import ConnectionsPanel from '../ConnectionsPanel';

describe('ConnectionsPanel — trust-surface polish', () => {
  it('renders all four connection options as buttons', () => {
    renderWithProviders(<ConnectionsPanel />);
    expect(screen.getByRole('button', { name: /Google/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Notion/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Web3 Wallet/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Crypto Trading Exchanges/ })).toBeInTheDocument();
  });

  it('shows "Coming soon" badge on every option (current product state)', () => {
    renderWithProviders(<ConnectionsPanel />);
    expect(screen.getAllByText(/Coming soon/i)).toHaveLength(4);
  });

  it('disables coming-soon rows so they are non-actionable', () => {
    renderWithProviders(<ConnectionsPanel />);
    const google = screen.getByRole('button', { name: /Google/ });
    expect(google).toBeDisabled();
  });

  it('renders the cross-surface trust notice with stone palette (not blue)', () => {
    const { container } = renderWithProviders(<ConnectionsPanel />);
    expect(screen.getByText('Privacy & Security')).toBeInTheDocument();
    // Polish guarantee: notice container uses the calm stone palette
    // matching PrivacyPanel's info box, not the louder blue we replaced.
    expect(container.querySelector('.bg-stone-50')).not.toBeNull();
    expect(container.querySelector('.bg-blue-50')).toBeNull();
  });
});
