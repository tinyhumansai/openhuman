import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import GlobalUpsellBanner from '../GlobalUpsellBanner';

const mockUseUsageState = vi.hoisted(() =>
  vi.fn(() => ({
    teamUsage: null as null | object,
    isLoading: false,
    isAtLimit: false,
    isNearLimit: false,
    isFreeTier: false,
    usagePct: 0,
  }))
);

vi.mock('../../../hooks/useUsageState', () => ({ useUsageState: mockUseUsageState }));

vi.mock('../../../utils/openUrl', () => ({ openUrl: vi.fn() }));

vi.mock('../UpsellBanner', () => ({
  default: ({ title, message, variant }: { title: string; message: string; variant: string }) => (
    <div data-testid="upsell-banner" data-variant={variant}>
      <span>{title}</span>
      <span>{message}</span>
    </div>
  ),
}));

const baseUsage = { cycleSpentUsd: 0, cycleBudgetUsd: 10, remainingUsd: 0 };

describe('GlobalUpsellBanner', () => {
  beforeEach(() => {
    mockUseUsageState.mockReset();
    mockUseUsageState.mockReturnValue({
      teamUsage: null,
      isLoading: false,
      isAtLimit: false,
      isNearLimit: false,
      isFreeTier: false,
      usagePct: 0,
    });
  });

  it('renders nothing when isLoading=true', () => {
    mockUseUsageState.mockReturnValue({
      teamUsage: baseUsage,
      isLoading: true,
      isAtLimit: false,
      isNearLimit: false,
      isFreeTier: false,
      usagePct: 0,
    });
    const { container } = render(<GlobalUpsellBanner />);
    expect(container.firstChild).toBeNull();
  });

  it('renders nothing when teamUsage is null', () => {
    const { container } = render(<GlobalUpsellBanner />);
    expect(container.firstChild).toBeNull();
  });

  it('renders nothing when not at limit and not near limit', () => {
    mockUseUsageState.mockReturnValue({
      teamUsage: baseUsage,
      isLoading: false,
      isAtLimit: false,
      isNearLimit: false,
      isFreeTier: true,
      usagePct: 0.5,
    });
    const { container } = render(<GlobalUpsellBanner />);
    expect(container.firstChild).toBeNull();
  });

  it('renders upgrade banner when isAtLimit=true', () => {
    mockUseUsageState.mockReturnValue({
      teamUsage: baseUsage,
      isLoading: false,
      isAtLimit: true,
      isNearLimit: true,
      isFreeTier: true,
      usagePct: 1,
    });
    render(<GlobalUpsellBanner />);
    const banner = screen.getByTestId('upsell-banner');
    expect(banner).toBeInTheDocument();
    expect(banner).toHaveAttribute('data-variant', 'upgrade');
  });

  it('renders warning banner with usage pct when isNearLimit=true and isFreeTier=true', () => {
    mockUseUsageState.mockReturnValue({
      teamUsage: baseUsage,
      isLoading: false,
      isAtLimit: false,
      isNearLimit: true,
      isFreeTier: true,
      usagePct: 0.85,
    });
    render(<GlobalUpsellBanner />);
    const banner = screen.getByTestId('upsell-banner');
    expect(banner).toHaveAttribute('data-variant', 'warning');
    // pct = Math.round(0.85 * 100) = 85; message should contain '85'
    expect(banner.textContent).toContain('85');
  });

  it('renders nothing when isNearLimit=true but isFreeTier=false (paid tier)', () => {
    mockUseUsageState.mockReturnValue({
      teamUsage: baseUsage,
      isLoading: false,
      isAtLimit: false,
      isNearLimit: true,
      isFreeTier: false,
      usagePct: 0.9,
    });
    const { container } = render(<GlobalUpsellBanner />);
    expect(container.firstChild).toBeNull();
  });
});
