import type { ApiResponse } from '../../types/api';
import { apiClient } from '../apiClient';

export interface CreditBalance {
  balanceUsd: number;
  topUpBalanceUsd: number;
  topUpBaselineUsd: number | null;
}

export interface TeamUsage {
  remainingUsd: number;
  cycleBudgetUsd: number;
  dailyUsage: number;
  totalInputTokensThisCycle: number;
  totalOutputTokensThisCycle: number;
  /** 5-hour rolling window: amount spent (USD) */
  fiveHourSpendUsd: number;
  /** 5-hour rolling window: cap for the user's plan (USD) */
  fiveHourCapUsd: number;
  /** ISO timestamp when the oldest 5-hour window entry expires (null if window is empty) */
  fiveHourResetsAt: string | null;
  /** ISO timestamp when the current weekly cycle started */
  cycleStartDate: string;
  /** ISO timestamp when the current weekly cycle ends */
  cycleEndsAt: string;
}

export interface TopUpResult {
  url: string;
  gatewayTransactionId: string;
  amountUsd: number;
  gateway: string;
}

export interface CreditTransaction {
  id: string;
  type: 'EARN' | 'SPEND';
  action: string;
  amountUsd: number;
  balanceAfterUsd: number;
  createdAt: string;
}

export interface PaginatedTransactions {
  transactions: CreditTransaction[];
  total: number;
}

// ── Auto-Recharge types ──────────────────────────────────────────────────────

export interface AutoRechargeSettings {
  enabled: boolean;
  thresholdUsd: number;
  rechargeAmountUsd: number;
  weeklyLimitUsd: number;
  spentThisWeekUsd: number;
  weekStartDate: string;
  inFlight: boolean;
  hasSavedPaymentMethod: boolean;
  lastTriggeredAt: string | null;
  lastRechargeAt: string | null;
  lastPaymentIntentId: string | null;
  lastError: string | null;
}

export interface AutoRechargeUpdatePayload {
  enabled?: boolean;
  thresholdUsd?: number;
  rechargeAmountUsd?: number;
  weeklyLimitUsd?: number;
}

export interface BillingAddress {
  line1?: string;
  city?: string;
  state?: string;
  postalCode?: string;
  country?: string;
}

export interface CardBillingDetails {
  name?: string;
  email?: string;
  address?: BillingAddress;
}

export interface SavedCard {
  id: string;
  brand: string;
  expMonth: number;
  expYear: number;
  isDefault: boolean;
  last4: string;
  billingDetails: CardBillingDetails;
}

export interface CardsData {
  customerId: string;
  defaultPaymentMethodId: string;
  cards: SavedCard[];
}

export interface SetupIntentData {
  clientSecret: string;
  customerId: string;
  setupIntentId: string;
}

export interface UpdateCardPayload {
  isDefault?: boolean;
  billingDetails?: CardBillingDetails;
}

/**
 * Credits API endpoints
 */
export const creditsApi = {
  /**
   * Get the current user's credit balance (general + top-up)
   * GET /credits/balance
   */
  getBalance: async (): Promise<CreditBalance> => {
    const response = await apiClient.get<ApiResponse<CreditBalance>>('/payments/credits/balance');
    return response.data;
  },

  /**
   * Get team inference budget usage for the current billing cycle
   * GET /teams/me/usage
   */
  getTeamUsage: async (): Promise<TeamUsage> => {
    const response = await apiClient.get<ApiResponse<TeamUsage>>('/teams/me/usage');
    return response.data;
  },

  /**
   * Start a top-up (get Stripe or Coinbase payment URL)
   * POST /credits/top-up
   */
  topUp: async (
    amountUsd: number,
    gateway: 'stripe' | 'coinbase' = 'stripe'
  ): Promise<TopUpResult> => {
    const response = await apiClient.post<ApiResponse<TopUpResult>>('/payments/credits/top-up', {
      amountUsd,
      gateway,
    });
    return response.data;
  },

  /**
   * Get paginated credit transaction history
   * GET /credits/transactions
   */
  getTransactions: async (limit = 20, offset = 0): Promise<PaginatedTransactions> => {
    const response = await apiClient.get<ApiResponse<PaginatedTransactions>>(
      `/credits/transactions?limit=${limit}&offset=${offset}`
    );
    return response.data;
  },

  // ── Auto-Recharge ──────────────────────────────────────────────────────────

  /**
   * Get auto-recharge settings
   * GET /payments/credits/auto-recharge
   */
  getAutoRecharge: async (): Promise<AutoRechargeSettings> => {
    const response = await apiClient.get<ApiResponse<AutoRechargeSettings>>(
      '/payments/credits/auto-recharge'
    );
    return response.data;
  },

  /**
   * Update auto-recharge settings. Enabling requires a saved card.
   * PATCH /payments/credits/auto-recharge
   */
  updateAutoRecharge: async (payload: AutoRechargeUpdatePayload): Promise<AutoRechargeSettings> => {
    const response = await apiClient.patch<ApiResponse<AutoRechargeSettings>>(
      '/payments/credits/auto-recharge',
      payload
    );
    return response.data;
  },

  /**
   * List saved cards for auto-recharge
   * GET /payments/credits/auto-recharge/cards
   */
  getCards: async (): Promise<CardsData> => {
    const response = await apiClient.get<ApiResponse<CardsData>>(
      '/payments/credits/auto-recharge/cards'
    );
    return response.data;
  },

  /**
   * Create a Stripe SetupIntent for adding a new card.
   * The returned clientSecret must be confirmed with Stripe.js.
   * POST /payments/credits/auto-recharge/cards/setup-intent
   */
  createSetupIntent: async (): Promise<SetupIntentData> => {
    const response = await apiClient.post<ApiResponse<SetupIntentData>>(
      '/payments/credits/auto-recharge/cards/setup-intent'
    );
    return response.data;
  },

  /**
   * Update a saved card (set as default or update billing details)
   * PATCH /payments/credits/auto-recharge/cards/:paymentMethodId
   */
  updateCard: async (paymentMethodId: string, payload: UpdateCardPayload): Promise<CardsData> => {
    const response = await apiClient.patch<ApiResponse<CardsData>>(
      `/payments/credits/auto-recharge/cards/${encodeURIComponent(paymentMethodId)}`,
      payload
    );
    return response.data;
  },

  /**
   * Remove a saved card. If it was the default, another card becomes default.
   * DELETE /payments/credits/auto-recharge/cards/:paymentMethodId
   */
  deleteCard: async (paymentMethodId: string): Promise<CardsData> => {
    const response = await apiClient.delete<ApiResponse<CardsData>>(
      `/payments/credits/auto-recharge/cards/${encodeURIComponent(paymentMethodId)}`
    );
    return response.data;
  },
};
