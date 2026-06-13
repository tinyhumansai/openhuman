/**
 * SubscriptionPlans — unit tests for the subscription plan selector.
 *
 * Target line: 53
 *
 * Line 53 — onCheckedChange of the crypto-toggle SettingsSwitch calls
 *           setPaymentMethod('crypto') or setPaymentMethod('card').
 *
 * NOTE: useT is NOT mocked here — default I18nContext (en.ts) is active.
 * All text assertions use actual English strings from en.ts.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { PlanTier } from '../../../../../types/api';
import SubscriptionPlans from '../SubscriptionPlans';

// ── Helpers ───────────────────────────────────────────────────────────────────

const defaultProps = {
  currentTier: 'FREE' as PlanTier,
  billingInterval: 'monthly' as const,
  setBillingInterval: vi.fn(),
  paymentMethod: 'card' as const,
  setPaymentMethod: vi.fn(),
  isPurchasing: false,
  purchasingTier: null,
  paymentConfirmed: false,
  onUpgrade: vi.fn(),
};

// ── Tests ─────────────────────────────────────────────────────────────────────

describe('SubscriptionPlans — crypto toggle (line 53)', () => {
  it('renders the crypto toggle switch', () => {
    render(<SubscriptionPlans {...defaultProps} />);
    // en.ts: 'settings.billing.subscription.cryptoQuestion': 'Want to pay with crypto?'
    expect(screen.getByText('Want to pay with crypto?')).toBeInTheDocument();
    // The switch element exists
    expect(screen.getByRole('switch')).toBeInTheDocument();
  });

  it('calls setPaymentMethod("crypto") when switch is toggled on (line 53)', () => {
    const setPaymentMethod = vi.fn();
    render(<SubscriptionPlans {...defaultProps} setPaymentMethod={setPaymentMethod} />);

    // Switch starts unchecked (paymentMethod='card')
    const toggle = screen.getByRole('switch');
    fireEvent.click(toggle);

    expect(setPaymentMethod).toHaveBeenCalledWith('crypto');
  });

  it('calls setPaymentMethod("card") when switch is toggled off', () => {
    const setPaymentMethod = vi.fn();
    render(
      <SubscriptionPlans
        {...defaultProps}
        paymentMethod="crypto"
        setPaymentMethod={setPaymentMethod}
      />
    );

    const toggle = screen.getByRole('switch');
    fireEvent.click(toggle);

    expect(setPaymentMethod).toHaveBeenCalledWith('card');
  });

  it('renders the monthly/annual billing interval buttons', () => {
    render(<SubscriptionPlans {...defaultProps} />);
    // en.ts: 'settings.billing.subscription.monthly': 'Monthly'
    // en.ts: 'settings.billing.subscription.annual': 'Annual'
    expect(screen.getByText('Monthly')).toBeInTheDocument();
    expect(screen.getByText('Annual')).toBeInTheDocument();
  });

  it('disables monthly button when crypto payment is selected', () => {
    render(<SubscriptionPlans {...defaultProps} paymentMethod="crypto" />);
    const monthlyBtn = screen.getByText('Monthly').closest('button');
    expect(monthlyBtn).toBeDisabled();
  });

  it('shows payment-confirmed banner when paymentConfirmed=true', () => {
    render(<SubscriptionPlans {...defaultProps} paymentConfirmed={true} />);
    // en.ts: 'settings.billing.subscription.paymentConfirmed': 'Payment confirmed'
    expect(screen.getByText('Payment confirmed')).toBeInTheDocument();
  });

  it('shows waiting-for-payment banner when isPurchasing=true', () => {
    render(
      <SubscriptionPlans {...defaultProps} isPurchasing={true} purchasingTier={'PRO' as PlanTier} />
    );
    // en.ts: 'settings.billing.subscription.waitingPayment': 'Waiting payment'
    expect(screen.getByText('Waiting payment')).toBeInTheDocument();
  });
});
