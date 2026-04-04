import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockCallCoreCommand = vi.fn();

vi.mock('../../coreCommandClient', () => ({
  callCoreCommand: (...args: unknown[]) => mockCallCoreCommand(...args),
}));

const { billingApi } = await import('../billingApi');

describe('billingApi', () => {
  beforeEach(() => {
    mockCallCoreCommand.mockReset();
  });

  describe('getCurrentPlan', () => {
    it('should call openhuman.billing_get_current_plan', async () => {
      const planData = {
        plan: 'BASIC',
        hasActiveSubscription: true,
        planExpiry: '2026-12-31T00:00:00.000Z',
        subscription: {
          id: 'sub_123',
          status: 'active',
          currentPeriodEnd: '2026-12-31T00:00:00.000Z',
          quantity: 1,
        },
        monthlyBudgetUsd: 20,
        weeklyBudgetUsd: 10,
        fiveHourCapUsd: 3,
      };
      mockCallCoreCommand.mockResolvedValue(planData);

      const result = await billingApi.getCurrentPlan();

      expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.billing_get_current_plan');
      expect(result).toEqual(planData);
    });

    it('should return FREE plan data for free users', async () => {
      const planData = {
        plan: 'FREE',
        hasActiveSubscription: false,
        planExpiry: null,
        subscription: null,
        monthlyBudgetUsd: 1,
        weeklyBudgetUsd: 0.5,
        fiveHourCapUsd: 0.15,
      };
      mockCallCoreCommand.mockResolvedValue(planData);

      const result = await billingApi.getCurrentPlan();

      expect(result.plan).toBe('FREE');
      expect(result.hasActiveSubscription).toBe(false);
      expect(result.subscription).toBeNull();
      expect(result.weeklyBudgetUsd).toBe(0.5);
    });

    it('should propagate errors', async () => {
      mockCallCoreCommand.mockRejectedValue(new Error('Unauthorized'));

      await expect(billingApi.getCurrentPlan()).rejects.toThrow('Unauthorized');
    });
  });

  describe('purchasePlan', () => {
    it('should call openhuman.billing_purchase_plan with plan ID', async () => {
      const checkoutData = {
        checkoutUrl: 'https://checkout.stripe.com/c/pay/cs_test_123',
        sessionId: 'cs_test_123',
      };
      mockCallCoreCommand.mockResolvedValue(checkoutData);

      const result = await billingApi.purchasePlan('BASIC_MONTHLY');

      expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.billing_purchase_plan', {
        plan: 'BASIC_MONTHLY',
      });
      expect(result).toEqual(checkoutData);
    });

    it('should pass yearly plan IDs correctly', async () => {
      mockCallCoreCommand.mockResolvedValue({
        checkoutUrl: 'https://stripe.com/...',
        sessionId: 'cs_456',
      });

      await billingApi.purchasePlan('PRO_YEARLY');

      expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.billing_purchase_plan', {
        plan: 'PRO_YEARLY',
      });
    });

    it('should return null checkoutUrl when session creation has no URL', async () => {
      mockCallCoreCommand.mockResolvedValue({ checkoutUrl: null, sessionId: 'cs_789' });

      const result = await billingApi.purchasePlan('BASIC_MONTHLY');

      expect(result.checkoutUrl).toBeNull();
      expect(result.sessionId).toBe('cs_789');
    });

    it('should propagate errors', async () => {
      mockCallCoreCommand.mockRejectedValue(new Error('Invalid plan'));

      await expect(billingApi.purchasePlan('BASIC_MONTHLY')).rejects.toThrow('Invalid plan');
    });
  });

  describe('createPortalSession', () => {
    it('should call openhuman.billing_create_portal_session', async () => {
      const portalData = { portalUrl: 'https://billing.stripe.com/p/session/test_123' };
      mockCallCoreCommand.mockResolvedValue(portalData);

      const result = await billingApi.createPortalSession();

      expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.billing_create_portal_session');
      expect(result).toEqual(portalData);
    });

    it('should return the portal URL string', async () => {
      mockCallCoreCommand.mockResolvedValue({
        portalUrl: 'https://billing.stripe.com/session/abc',
      });

      const result = await billingApi.createPortalSession();

      expect(result.portalUrl).toBe('https://billing.stripe.com/session/abc');
    });

    it('should propagate errors', async () => {
      mockCallCoreCommand.mockRejectedValue(new Error('Unable to resolve Stripe customer'));

      await expect(billingApi.createPortalSession()).rejects.toThrow(
        'Unable to resolve Stripe customer'
      );
    });
  });

  describe('createCoinbaseCharge', () => {
    it('should call openhuman.billing_create_coinbase_charge with plan and interval', async () => {
      const chargeData = {
        gatewayTransactionId: 'charge_abc',
        hostedUrl: 'https://commerce.coinbase.com/charges/abc',
        status: 'created',
        expiresAt: '2026-01-31T12:15:00.000Z',
      };
      mockCallCoreCommand.mockResolvedValue(chargeData);

      const result = await billingApi.createCoinbaseCharge('BASIC', 'annual');

      expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.billing_create_coinbase_charge', {
        plan: 'BASIC',
        interval: 'annual',
      });
      expect(result).toEqual(chargeData);
    });

    it('should default interval to annual', async () => {
      mockCallCoreCommand.mockResolvedValue({
        gatewayTransactionId: 'charge_xyz',
        hostedUrl: 'https://commerce.coinbase.com/charges/xyz',
        status: 'created',
        expiresAt: '2026-01-31T12:15:00.000Z',
      });

      await billingApi.createCoinbaseCharge('PRO');

      expect(mockCallCoreCommand).toHaveBeenCalledWith('openhuman.billing_create_coinbase_charge', {
        plan: 'PRO',
        interval: 'annual',
      });
    });

    it('should return hosted URL for payment redirect', async () => {
      const expectedUrl = 'https://commerce.coinbase.com/charges/test';
      mockCallCoreCommand.mockResolvedValue({
        gatewayTransactionId: 'charge_t',
        hostedUrl: expectedUrl,
        status: 'created',
        expiresAt: '2026-01-31T12:15:00.000Z',
      });

      const result = await billingApi.createCoinbaseCharge('BASIC', 'annual');

      expect(result.hostedUrl).toBe(expectedUrl);
    });

    it('should propagate errors', async () => {
      mockCallCoreCommand.mockRejectedValue(
        new Error('Crypto payments are only available for annual plans')
      );

      await expect(billingApi.createCoinbaseCharge('BASIC', 'annual')).rejects.toThrow(
        'Crypto payments are only available for annual plans'
      );
    });
  });
});
