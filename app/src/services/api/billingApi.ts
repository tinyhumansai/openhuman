import type {
  CoinbaseChargeData,
  CurrentPlanData,
  PlanIdentifier,
  PlanTier,
  PortalSessionData,
  PurchasePlanData,
} from '../../types/api';
import { callCoreCommand } from '../coreCommandClient';

/**
 * Billing API endpoints
 */
export const billingApi = {
  /**
   * Get the current user's subscription plan
   * GET /payments/stripe/currentPlan
   */
  getCurrentPlan: async (): Promise<CurrentPlanData> => {
    return await callCoreCommand<CurrentPlanData>('openhuman.billing_get_current_plan');
  },

  /**
   * Create a Stripe Checkout session for a plan purchase
   * POST /payments/stripe/purchasePlan
   */
  purchasePlan: async (plan: PlanIdentifier): Promise<PurchasePlanData> => {
    return await callCoreCommand<PurchasePlanData>('openhuman.billing_purchase_plan', { plan });
  },

  /**
   * Create a Stripe Customer Portal session
   * POST /payments/stripe/portal
   */
  createPortalSession: async (): Promise<PortalSessionData> => {
    return await callCoreCommand<PortalSessionData>('openhuman.billing_create_portal_session');
  },

  /**
   * Create a Coinbase Commerce charge (annual-only)
   * POST /payments/coinbase/charge
   */
  createCoinbaseCharge: async (
    plan: PlanTier,
    interval: 'annual' = 'annual'
  ): Promise<CoinbaseChargeData> => {
    return await callCoreCommand<CoinbaseChargeData>('openhuman.billing_create_coinbase_charge', {
      plan,
      interval,
    });
  },
};
