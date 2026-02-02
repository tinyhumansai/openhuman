import type {
  ApiResponse,
  CoinbaseChargeData,
  CurrentPlanData,
  PlanIdentifier,
  PlanTier,
  PortalSessionData,
  PurchasePlanData,
} from '../../types/api';
import { apiClient } from '../apiClient';

/**
 * Billing API endpoints
 */
export const billingApi = {
  /**
   * Get the current user's subscription plan
   * GET /payments/stripe/currentPlan
   */
  getCurrentPlan: async (): Promise<CurrentPlanData> => {
    const response = await apiClient.get<ApiResponse<CurrentPlanData>>(
      '/payments/stripe/currentPlan'
    );
    return response.data;
  },

  /**
   * Create a Stripe Checkout session for a plan purchase
   * POST /payments/stripe/purchasePlan
   */
  purchasePlan: async (plan: PlanIdentifier): Promise<PurchasePlanData> => {
    const response = await apiClient.post<ApiResponse<PurchasePlanData>>(
      '/payments/stripe/purchasePlan',
      { plan }
    );
    return response.data;
  },

  /**
   * Create a Stripe Customer Portal session
   * POST /payments/stripe/portal
   */
  createPortalSession: async (): Promise<PortalSessionData> => {
    const response =
      await apiClient.post<ApiResponse<PortalSessionData>>('/payments/stripe/portal');
    return response.data;
  },

  /**
   * Create a Coinbase Commerce charge (annual-only)
   * POST /payments/coinbase/charge
   */
  createCoinbaseCharge: async (
    plan: PlanTier,
    interval: 'annual' = 'annual'
  ): Promise<CoinbaseChargeData> => {
    const response = await apiClient.post<ApiResponse<CoinbaseChargeData>>(
      '/payments/coinbase/charge',
      { plan, interval }
    );
    return response.data;
  },
};
