import { callCoreCommand } from '../coreCommandClient';

/**
 * Credit balance payload returned by `GET /payments/credits/balance`.
 *
 * Mirrors the backend shape defined in
 * `backend-1/src/services/user/balanceService.ts` → `getCreditBalance(userId)`,
 * which in turn derives from `IUser.usage.promotionBalanceUsd` on the user
 * model and the team-level top-up ledger.
 */
export interface CreditBalance {
  /**
   * Promotional credit balance on the user document (signup bonus, coupons,
   * referral rewards). Corresponds to `IUserUsage.promotionBalanceUsd`.
   */
  promotionBalanceUsd: number;
  /**
   * Team-level top-up balance (paid credits that cover overage once the
   * included cycle budget is exhausted). Returned by `getTeamTopup(userId)`.
   */
  teamTopupUsd: number;
}

export interface TeamUsage {
  remainingUsd: number;
  cycleBudgetUsd: number;
  /** Amount spent in the current 5-hour fixed window (USD) */
  cycleLimit5hr: number;
  /** Amount spent in the current 7-day cycle (USD) */
  cycleLimit7day: number;
  /** Max USD allowed in the 5-hour window for the current subscription tier */
  fiveHourCapUsd: number;
  /** ISO timestamp when the 5-hour window resets (null if window is empty) */
  fiveHourResetsAt: string | null;
  /** ISO timestamp when the current weekly cycle started */
  cycleStartDate: string;
  /** ISO timestamp when the current weekly cycle ends */
  cycleEndsAt: string;
  /** When true, cycle limits are not enforced for this user (test/internal accounts) */
  bypassCycleLimit?: boolean;
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
  couponCode: string;
  amountUsd: number;
  pending: boolean;
}

export interface RedeemedCoupon {
  code: string;
  amountUsd: number;
  redeemedAt: string | null;
  activationType: string;
  fulfilled: boolean;
  fulfilledAt: string | null;
  activationCondition: string | null;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function asNumber(value: unknown): number {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string' && value.trim() !== '') {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return 0;
}

function asStringOrNull(value: unknown): string | null {
  return typeof value === 'string' && value.trim() !== '' ? value : null;
}

export function normalizeCouponRedeemResult(raw: unknown): CouponRedeemResult {
  const record = asRecord(raw) ?? {};
  const envelopeData = asRecord(record.data);
  const payload = envelopeData ?? record;
  return {
    couponCode:
      (typeof payload.couponCode === 'string' && payload.couponCode.trim()) ||
      (typeof payload.code === 'string' && payload.code.trim()) ||
      '',
    amountUsd: asNumber(payload.amountUsd ?? payload.amount_usd),
    pending: Boolean(payload.pending),
  };
}

export function normalizeRedeemedCoupon(raw: unknown): RedeemedCoupon {
  const record = asRecord(raw) ?? {};
  return {
    code:
      (typeof record.code === 'string' && record.code.trim()) ||
      (typeof record.couponCode === 'string' && record.couponCode.trim()) ||
      '',
    amountUsd: asNumber(record.amountUsd ?? record.amount_usd),
    redeemedAt: asStringOrNull(record.redeemedAt ?? record.redeemed_at),
    activationType:
      (typeof record.activationType === 'string' && record.activationType.trim()) ||
      (typeof record.activation_type === 'string' && record.activation_type.trim()) ||
      'IMMEDIATE',
    fulfilled: Boolean(record.fulfilled),
    fulfilledAt: asStringOrNull(record.fulfilledAt ?? record.fulfilled_at),
    activationCondition: asStringOrNull(record.activationCondition ?? record.activation_condition),
  };
}

function normalizeUsd(value: unknown, fallback = 0): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}

function normalizeCreditBalance(payload: unknown): CreditBalance {
  const raw = (payload && typeof payload === 'object' ? payload : {}) as Record<string, unknown>;

  return {
    promotionBalanceUsd: normalizeUsd(raw.promotionBalanceUsd),
    teamTopupUsd: normalizeUsd(raw.teamTopupUsd),
  };
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
    const result = await callCoreCommand<CreditBalance>('openhuman.billing_get_balance');
    return normalizeCreditBalance(result);
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
    const result = await callCoreCommand<unknown>('openhuman.billing_redeem_coupon', { code });
    return normalizeCouponRedeemResult(result);
  },

  /**
   * List coupons redeemed by the current user.
   * GET /coupons/me
   */
  getUserCoupons: async (): Promise<RedeemedCoupon[]> => {
    const coupons = await callCoreCommand<unknown[]>('openhuman.billing_get_coupons');
    return Array.isArray(coupons) ? coupons.map(normalizeRedeemedCoupon) : [];
  },
};
