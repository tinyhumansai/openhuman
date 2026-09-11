import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import BillingPanel from './BillingPanel';

const navigateBack = vi.fn();

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack,
    navigateToSettings: vi.fn(),
    navigateToTeamManagement: vi.fn(),
    breadcrumbs: [],
  }),
}));

const openUrlMock = vi.fn();
vi.mock('../../../utils/openUrl', () => ({ openUrl: (url: string) => openUrlMock(url) }));

const getCurrentPlanMock = vi.fn();
const purchasePlanMock = vi.fn();
const createCoinbaseChargeMock = vi.fn();

vi.mock('../../../services/api/billingApi', () => ({
  billingApi: {
    getCurrentPlan: (...args: unknown[]) => getCurrentPlanMock(...args),
    purchasePlan: (...args: unknown[]) => purchasePlanMock(...args),
    createCoinbaseCharge: (...args: unknown[]) => createCoinbaseChargeMock(...args),
  },
}));

describe('<BillingPanel />', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    openUrlMock.mockResolvedValue(undefined);
    getCurrentPlanMock.mockResolvedValue({
      plan: 'FREE',
      hasActiveSubscription: false,
      planExpiry: null,
      subscription: null,
      monthlyBudgetUsd: 0,
      weeklyBudgetUsd: 0,
    });
    purchasePlanMock.mockResolvedValue({
      checkoutUrl: 'https://checkout.stripe.com/test',
      sessionId: 'test-session',
    });
    createCoinbaseChargeMock.mockResolvedValue({
      gatewayTransactionId: 'test-gw',
      hostedUrl: 'https://commerce.coinbase.com/test',
      status: 'NEW',
      expiresAt: '2026-01-01T00:00:00Z',
    });
  });

  it('renders the plan selector and the dashboard button without auto-opening the browser', async () => {
    render(<BillingPanel />);

    // SubscriptionPlans renders its own title; billing frequency selection is
    // back in-app so users can change their plan without leaving the desktop app.
    expect(screen.getByText('Choose a Plan')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Open billing dashboard' })).toBeInTheDocument();

    // getCurrentPlan is called on mount but must not trigger a browser open.
    await waitFor(() => expect(getCurrentPlanMock).toHaveBeenCalledTimes(1));
    expect(openUrlMock).not.toHaveBeenCalled();
  });

  it('loads the current plan tier on mount and passes it to SubscriptionPlans', async () => {
    getCurrentPlanMock.mockResolvedValue({
      plan: 'BASIC',
      hasActiveSubscription: true,
      planExpiry: null,
      subscription: null,
      monthlyBudgetUsd: 20,
      weeklyBudgetUsd: 10,
    });

    render(<BillingPanel />);

    await waitFor(() => expect(getCurrentPlanMock).toHaveBeenCalledTimes(1));
    // With BASIC as current tier the BASIC card shows the "Current plan" badge.
    expect(await screen.findByText('Current plan')).toBeInTheDocument();
  });

  it('upgrade with card payment calls purchasePlan and opens the checkout URL', async () => {
    render(<BillingPanel />);

    await waitFor(() => expect(getCurrentPlanMock).toHaveBeenCalledTimes(1));

    // Both BASIC and PRO show upgrade buttons when current tier is FREE.
    const upgradeButtons = screen.getAllByRole('button', { name: 'Upgrade' });
    fireEvent.click(upgradeButtons[0]);

    await waitFor(() => expect(purchasePlanMock).toHaveBeenCalledTimes(1));
    expect(purchasePlanMock).toHaveBeenCalledWith('BASIC_MONTHLY');
    await waitFor(() =>
      expect(openUrlMock).toHaveBeenCalledWith('https://checkout.stripe.com/test')
    );
  });

  // The reason this PR exists: the interval toggle must reach `purchasePlan`.
  // The monthly case above passes on the DEFAULT interval, so it stays green
  // even if `buildPlanId(tier, billingInterval)` is hardcoded back to
  // 'monthly' — i.e. even with the bug in #5865 fully restored. This is the
  // case that fails when that happens.
  it('upgrade after selecting Annual sends the yearly plan id', async () => {
    render(<BillingPanel />);
    await waitFor(() => expect(getCurrentPlanMock).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole('button', { name: 'Annual' }));

    const upgradeButtons = await screen.findAllByRole('button', { name: 'Upgrade' });
    fireEvent.click(upgradeButtons[0]);

    await waitFor(() => expect(purchasePlanMock).toHaveBeenCalledTimes(1));
    expect(purchasePlanMock).toHaveBeenCalledWith('BASIC_YEARLY');
  });

  // The crypto branch of `handleUpgrade` had no test at all: the mock was
  // declared and stubbed but never asserted on, so the whole branch was
  // unexecuted. Also pins the interval coupling from the Codex P1 — selecting
  // crypto forces `annual`, so the price on screen matches the charge.
  it('upgrade with crypto creates a Coinbase charge and opens the hosted URL', async () => {
    render(<BillingPanel />);
    await waitFor(() => expect(getCurrentPlanMock).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole('switch'));

    const upgradeButtons = await screen.findAllByRole('button', { name: 'Upgrade' });
    fireEvent.click(upgradeButtons[0]);

    await waitFor(() => expect(createCoinbaseChargeMock).toHaveBeenCalledTimes(1));
    expect(createCoinbaseChargeMock).toHaveBeenCalledWith('BASIC');
    // Crypto must never go through the Stripe path.
    expect(purchasePlanMock).not.toHaveBeenCalled();
    await waitFor(() =>
      expect(openUrlMock).toHaveBeenCalledWith('https://commerce.coinbase.com/test')
    );
    // Selecting crypto switches the interval to annual, so the monthly
    // button is disabled and the displayed price cannot disagree with the
    // charge that was created.
    expect(screen.getByRole('button', { name: 'Monthly' })).toBeDisabled();
  });

  it('opens the billing dashboard when the user clicks the secondary button', async () => {
    render(<BillingPanel />);

    fireEvent.click(screen.getByRole('button', { name: 'Open billing dashboard' }));
    await waitFor(() => expect(openUrlMock).toHaveBeenCalledTimes(1));
    expect(openUrlMock).toHaveBeenLastCalledWith('https://tinyhumans.ai/dashboard');
  });

  it('invokes the navigation back handler from both the header and the inline button', async () => {
    render(<BillingPanel />);

    // The SettingsHeader back button (aria-label "Back") and the inline
    // "Back to settings" button both route through navigateBack.
    fireEvent.click(screen.getByRole('button', { name: 'Back' }));
    fireEvent.click(screen.getByRole('button', { name: 'Back to settings' }));
    expect(navigateBack).toHaveBeenCalledTimes(2);
  });

  it('shows an error message when getCurrentPlan rejects', async () => {
    getCurrentPlanMock.mockRejectedValue(new Error('Network error'));

    render(<BillingPanel />);

    await waitFor(() => expect(screen.getByText('Network error')).toBeInTheDocument());
  });

  it('shows an error message when purchasePlan rejects', async () => {
    purchasePlanMock.mockRejectedValue(new Error('Payment failed'));

    render(<BillingPanel />);
    await waitFor(() => expect(getCurrentPlanMock).toHaveBeenCalledTimes(1));

    const upgradeButtons = screen.getAllByRole('button', { name: 'Upgrade' });
    fireEvent.click(upgradeButtons[0]);

    await waitFor(() => expect(screen.getByText('Payment failed')).toBeInTheDocument());
  });

  it('shows an error when purchasePlan returns no checkout URL', async () => {
    purchasePlanMock.mockResolvedValue({ checkoutUrl: null, sessionId: 'test-session' });

    render(<BillingPanel />);
    await waitFor(() => expect(getCurrentPlanMock).toHaveBeenCalledTimes(1));

    const upgradeButtons = screen.getAllByRole('button', { name: 'Upgrade' });
    fireEvent.click(upgradeButtons[0]);

    await waitFor(() =>
      expect(screen.getByText('Checkout session did not return a redirect URL')).toBeInTheDocument()
    );
    expect(openUrlMock).not.toHaveBeenCalled();
  });
});
