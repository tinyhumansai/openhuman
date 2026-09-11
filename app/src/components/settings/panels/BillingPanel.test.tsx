import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import BillingPanel from './BillingPanel';

const navigateBack = vi.fn();
const openUrlMock = vi.fn();
const getSummaryMock = vi.fn();
const getTeamUsageMock = vi.fn();

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack,
    navigateToSettings: vi.fn(),
    navigateToTeamManagement: vi.fn(),
    breadcrumbs: [],
  }),
}));

vi.mock('../../../utils/openUrl', () => ({ openUrl: (url: string) => openUrlMock(url) }));
vi.mock('../../../services/api/billingApi', () => ({
  billingApi: { getSummary: (...args: unknown[]) => getSummaryMock(...args) },
}));
vi.mock('../../../services/api/creditsApi', () => ({
  creditsApi: { getTeamUsage: (...args: unknown[]) => getTeamUsageMock(...args) },
}));

const summary = {
  credits: { promotionBalanceUsd: 4.5, teamTopupUsd: 10, totalUsd: 14.5 },
  plan: {
    plan: 'PRO',
    hasActiveSubscription: true,
    planExpiry: '2026-12-01T00:00:00.000Z',
    subscription: null,
    monthlyBudgetUsd: 100,
    weeklyBudgetUsd: 25,
  },
  links: {
    topUpUrl: 'https://staging.tinyhumans.ai/dashboard?tab=billing',
    manageUrl: 'https://staging.tinyhumans.ai/dashboard?tab=plans',
    apiKeysUrl: 'https://staging.tinyhumans.ai/dashboard?tab=api-keys',
  },
};

const usage = {
  remainingUsd: 39.5,
  cycleBudgetUsd: 25,
  cycleSpentUsd: 6,
  cycleStartDate: '2026-09-07T00:00:00.000Z',
  cycleEndsAt: '2026-09-14T00:00:00.000Z',
  plan: {
    plan: 'PRO',
    name: 'Pro',
    marginPercent: 10,
    payAsYouGoMarginPercent: 100,
    discountVsPayAsYouGoPercent: 90,
  },
  insights: {
    period: { startDate: '2026-09-07', endDate: '2026-09-14' },
    totals: {
      inferenceUsd: 5,
      integrationsUsd: 1,
      totalUsd: 6,
      inferenceCalls: 20,
      integrationCalls: 2,
    },
    dailySeries: [],
    topModels: [],
    topIntegrations: [],
  },
};

describe('<BillingPanel />', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    openUrlMock.mockResolvedValue(undefined);
    getSummaryMock.mockResolvedValue(summary);
    getTeamUsageMock.mockResolvedValue(usage);
  });

  it('shows plan, balances, cycle spend, and total funds remaining', async () => {
    render(<BillingPanel />);

    await waitFor(() => expect(getSummaryMock).toHaveBeenCalledTimes(1));
    expect(getTeamUsageMock).toHaveBeenCalledTimes(1);
    expect(screen.getByText('PRO')).toBeInTheDocument();
    expect(screen.getByText('$39.50')).toBeInTheDocument();
    expect(screen.getByText('$4.50')).toBeInTheDocument();
    expect(screen.getByText('$10.00')).toBeInTheDocument();
    expect(screen.getByText(/Spent \$6\.00 this cycle/i)).toBeInTheDocument();
  });

  it('shows each response without waiting for the other request to settle', async () => {
    let resolveUsage: (value: typeof usage) => void = () => undefined;
    getTeamUsageMock.mockReturnValue(
      new Promise<typeof usage>(resolve => {
        resolveUsage = resolve;
      })
    );
    render(<BillingPanel />);

    expect(await screen.findByText('$4.50')).toBeInTheDocument();
    expect(screen.getByText('$10.00')).toBeInTheDocument();
    expect(screen.queryByText('$39.50')).not.toBeInTheDocument();

    await act(async () => resolveUsage(usage));
    expect(await screen.findByText('$39.50')).toBeInTheDocument();
  });

  it('uses backend-provided dashboard URLs for billing actions', async () => {
    render(<BillingPanel />);
    await screen.findByText('$39.50');

    fireEvent.click(screen.getByRole('button', { name: 'Top Up Credits' }));
    fireEvent.click(screen.getByRole('button', { name: 'Open billing dashboard' }));

    expect(openUrlMock).toHaveBeenNthCalledWith(1, summary.links.topUpUrl);
    expect(openUrlMock).toHaveBeenNthCalledWith(2, summary.links.manageUrl);
  });

  it('keeps usable cycle details visible when the aggregate summary fails', async () => {
    getSummaryMock.mockRejectedValue(new Error('Summary unavailable'));
    render(<BillingPanel />);

    expect(await screen.findByText('Summary unavailable')).toBeInTheDocument();
    expect(screen.getByText('PRO')).toBeInTheDocument();
    expect(screen.getByText('$39.50')).toBeInTheDocument();
    expect(screen.getByText(/Spent \$6\.00 this cycle/i)).toBeInTheDocument();
  });

  it('renders malformed balances as unavailable and falls back from unusable links', async () => {
    getSummaryMock.mockResolvedValue({
      ...summary,
      credits: {
        promotionBalanceUsd: Number.NaN,
        teamTopupUsd: -1,
        totalUsd: Number.POSITIVE_INFINITY,
      },
      links: null,
    });
    render(<BillingPanel />);

    await screen.findByText('$39.50');
    expect(screen.getAllByText('n/a')).toHaveLength(2);

    fireEvent.click(screen.getByRole('button', { name: 'Top Up Credits' }));
    fireEvent.click(screen.getByRole('button', { name: 'Open billing dashboard' }));
    expect(openUrlMock).toHaveBeenNthCalledWith(1, 'https://tinyhumans.ai/dashboard?tab=billing');
    expect(openUrlMock).toHaveBeenNthCalledWith(2, 'https://tinyhumans.ai/dashboard');
  });

  it('falls back when the backend returns empty billing links', async () => {
    getSummaryMock.mockResolvedValue({
      ...summary,
      links: { ...summary.links, topUpUrl: '', manageUrl: '' },
    });
    render(<BillingPanel />);

    await screen.findByText('$39.50');
    fireEvent.click(screen.getByRole('button', { name: 'Top Up Credits' }));
    fireEvent.click(screen.getByRole('button', { name: 'Open billing dashboard' }));
    expect(openUrlMock).toHaveBeenNthCalledWith(1, 'https://tinyhumans.ai/dashboard?tab=billing');
    expect(openUrlMock).toHaveBeenNthCalledWith(2, 'https://tinyhumans.ai/dashboard');
  });

  it('keeps plan and balances visible when cycle usage fails', async () => {
    getTeamUsageMock.mockRejectedValue(new Error('Usage unavailable'));
    render(<BillingPanel />);

    expect(await screen.findByText('Usage unavailable')).toBeInTheDocument();
    expect(screen.getByText('PRO')).toBeInTheDocument();
    expect(screen.queryByText('$14.50')).not.toBeInTheDocument();
    expect(screen.getAllByText('n/a').length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText(/Unable to load usage data/i)).toBeInTheDocument();
  });

  it('invokes the navigation back handler from both back buttons', async () => {
    render(<BillingPanel />);
    await screen.findByText('$39.50');

    fireEvent.click(screen.getByRole('button', { name: 'Back' }));
    fireEvent.click(screen.getByRole('button', { name: 'Back to settings' }));
    expect(navigateBack).toHaveBeenCalledTimes(2);
  });
});
