import { callCoreCommand } from '../coreCommandClient';

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

// ── Coupon types ────────────────────────────────────────────────────────────

export interface CouponRedeemResult {
  success: boolean;
  data: { code: string; amountUsd: number };
}

export interface RedeemedCoupon {
  code: string;
  amountUsd: number;
  redeemedAt: string;
  activationType: string;
  fulfilled: boolean;
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
    return await callCoreCommand<CreditBalance>('openhuman.billing_get_balance');
  },

  /**
   * Get team inference budget usage for the current billing cycle
   * GET /teams/me/usage
   */
  getTeamUsage: async (): Promise<TeamUsage> => {
    return await callCoreCommand<TeamUsage>('openhuman.team_get_usage');
  },

  /**
   * Start a top-up (get Stripe or Coinbase payment URL)
   * POST /credits/top-up
   */
  topUp: async (
    amountUsd: number,
    gateway: 'stripe' | 'coinbase' = 'stripe'
  ): Promise<TopUpResult> => {
    return await callCoreCommand<TopUpResult>('openhuman.billing_top_up', { amountUsd, gateway });
  },

  /**
   * Get paginated credit transaction history
   * GET /credits/transactions
   */
  getTransactions: async (limit = 20, offset = 0): Promise<PaginatedTransactions> => {
    return await callCoreCommand<PaginatedTransactions>('openhuman.billing_get_transactions', {
      limit,
      offset,
    });
  },

  // ── Auto-Recharge ──────────────────────────────────────────────────────────

  /**
   * Get auto-recharge settings
   * GET /payments/credits/auto-recharge
   */
  getAutoRecharge: async (): Promise<AutoRechargeSettings> => {
    return await callCoreCommand<AutoRechargeSettings>('openhuman.billing_get_auto_recharge');
  },

  /**
   * Update auto-recharge settings. Enabling requires a saved card.
   * PATCH /payments/credits/auto-recharge
   */
  updateAutoRecharge: async (payload: AutoRechargeUpdatePayload): Promise<AutoRechargeSettings> => {
    return await callCoreCommand<AutoRechargeSettings>('openhuman.billing_update_auto_recharge', {
      payload,
    });
  },

  /**
   * List saved cards for auto-recharge
   * GET /payments/credits/auto-recharge/cards
   */
  getCards: async (): Promise<CardsData> => {
    return await callCoreCommand<CardsData>('openhuman.billing_get_cards');
  },

  /**
   * Create a Stripe SetupIntent for adding a new card.
   * The returned clientSecret must be confirmed with Stripe.js.
   * POST /payments/credits/auto-recharge/cards/setup-intent
   */
  createSetupIntent: async (): Promise<SetupIntentData> => {
    return await callCoreCommand<SetupIntentData>('openhuman.billing_create_setup_intent');
  },

  /**
   * Update a saved card (set as default or update billing details)
   * PATCH /payments/credits/auto-recharge/cards/:paymentMethodId
   */
  updateCard: async (paymentMethodId: string, payload: UpdateCardPayload): Promise<CardsData> => {
    return await callCoreCommand<CardsData>('openhuman.billing_update_card', {
      paymentMethodId,
      payload,
    });
  },

  /**
   * Remove a saved card. If it was the default, another card becomes default.
   * DELETE /payments/credits/auto-recharge/cards/:paymentMethodId
   */
  deleteCard: async (paymentMethodId: string): Promise<CardsData> => {
    return await callCoreCommand<CardsData>('openhuman.billing_delete_card', { paymentMethodId });
  },

  // ── Coupons ──────────────────────────────────────────────────────────────

  /**
   * Redeem a coupon code to add credits.
   * POST /coupons/redeem
   */
  redeemCoupon: async (code: string): Promise<CouponRedeemResult> => {
    return await callCoreCommand<CouponRedeemResult>('openhuman.billing_redeem_coupon', { code });
  },

  /**
   * List coupons redeemed by the current user.
   * GET /coupons/me
   */
  getUserCoupons: async (): Promise<RedeemedCoupon[]> => {
    return await callCoreCommand<RedeemedCoupon[]>('openhuman.billing_get_coupons');
  },
};
