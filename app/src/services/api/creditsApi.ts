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

export interface TeamUsagePlanSummary {
  plan: string;
  name: string;
  marginPercent: number;
  payAsYouGoMarginPercent: number;
  discountVsPayAsYouGoPercent: number;
}

export interface TeamUsageDailyPoint {
  date: string;
  inferenceUsd: number;
  integrationsUsd: number;
  totalUsd: number;
}

export interface TeamUsageModelRow {
  model: string;
  provider: string;
  spentUsd: number;
  calls: number;
}

export interface TeamUsageIntegrationRow {
  provider: string;
  action: string;
  spentUsd: number;
  calls: number;
}

export interface TeamUsageInsights {
  period: { startDate: string; endDate: string };
  totals: {
    inferenceUsd: number;
    integrationsUsd: number;
    totalUsd: number;
    inferenceCalls: number;
    integrationCalls: number;
  };
  dailySeries: TeamUsageDailyPoint[];
  topModels: TeamUsageModelRow[];
  topIntegrations: TeamUsageIntegrationRow[];
}

/**
 * Cycle budget snapshot returned by `GET /teams/me/usage`. Backend PR #790
 * dropped rate-limit fields (5-hour window, daily caps); enforcement is now
 * purely budget-based via `BalanceService.canAfford`.
 */
export interface TeamUsage {
  remainingUsd: number;
  cycleBudgetUsd: number;
  cycleSpentUsd: number;
  cycleStartDate: string;
  cycleEndsAt: string;
  plan: TeamUsagePlanSummary;
  insights: TeamUsageInsights;
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

function normalizeUsd(value: unknown, fallback = 0): number {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string' && value.trim() !== '') {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return fallback;
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
    amountUsd: normalizeUsd(payload.amountUsd ?? payload.amount_usd),
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
    amountUsd: normalizeUsd(record.amountUsd ?? record.amount_usd),
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

function normalizeCreditBalance(payload: unknown): CreditBalance {
  const raw = (payload && typeof payload === 'object' ? payload : {}) as Record<string, unknown>;
  const nested = asRecord(raw.data) ?? asRecord(raw.balance) ?? null;
  const source = nested ?? raw;
  const promotionBalanceKeys = [
    'promotionBalanceUsd',
    'promotion_balance_usd',
    'promotionalBalanceUsd',
    'promotional_balance_usd',
    'promoBalanceUsd',
    'promo_balance_usd',
  ] as const;
  const teamTopupKeys = [
    'teamTopupUsd',
    'team_topup_usd',
    'teamTopUpUsd',
    'team_top_up_usd',
    'teamTopupBalanceUsd',
    'team_topup_balance_usd',
  ] as const;
  const missingPromotionBalance = promotionBalanceKeys.every(key => !(key in source));
  const missingTeamTopup = teamTopupKeys.every(key => !(key in source));

  if (missingPromotionBalance || missingTeamTopup) {
    console.debug('[creditsApi] normalizeCreditBalance missing expected keys', {
      raw: source,
      missingPromotionBalance,
      missingPromotionBalanceKeys: missingPromotionBalance ? promotionBalanceKeys : [],
      missingTeamTopup,
      missingTeamTopupKeys: missingTeamTopup ? teamTopupKeys : [],
    });
  }

  return {
    promotionBalanceUsd: normalizeUsd(
      source.promotionBalanceUsd ??
        source.promotion_balance_usd ??
        source.promotionalBalanceUsd ??
        source.promotional_balance_usd ??
        source.promoBalanceUsd ??
        source.promo_balance_usd
    ),
    teamTopupUsd: normalizeUsd(
      source.teamTopupUsd ??
        source.team_topup_usd ??
        source.teamTopUpUsd ??
        source.team_top_up_usd ??
        source.teamTopupBalanceUsd ??
        source.team_topup_balance_usd
    ),
  };
}

function normalizePlanSummary(raw: unknown): TeamUsagePlanSummary {
  const r = asRecord(raw) ?? {};
  return {
    plan: typeof r.plan === 'string' ? r.plan : 'FREE',
    name: typeof r.name === 'string' ? r.name : '',
    marginPercent: normalizeUsd(r.marginPercent),
    payAsYouGoMarginPercent: normalizeUsd(r.payAsYouGoMarginPercent),
    discountVsPayAsYouGoPercent: normalizeUsd(r.discountVsPayAsYouGoPercent),
  };
}

function normalizeDailyPoint(raw: unknown): TeamUsageDailyPoint {
  const r = asRecord(raw) ?? {};
  return {
    date: typeof r.date === 'string' ? r.date : '',
    inferenceUsd: normalizeUsd(r.inferenceUsd),
    integrationsUsd: normalizeUsd(r.integrationsUsd),
    totalUsd: normalizeUsd(r.totalUsd),
  };
}

function normalizeModelRow(raw: unknown): TeamUsageModelRow {
  const r = asRecord(raw) ?? {};
  return {
    model: typeof r.model === 'string' ? r.model : '',
    provider: typeof r.provider === 'string' ? r.provider : '',
    spentUsd: normalizeUsd(r.spentUsd),
    calls: Math.round(Number(r.calls) || 0),
  };
}

function normalizeIntegrationRow(raw: unknown): TeamUsageIntegrationRow {
  const r = asRecord(raw) ?? {};
  return {
    provider: typeof r.provider === 'string' ? r.provider : '',
    action: typeof r.action === 'string' ? r.action : '',
    spentUsd: normalizeUsd(r.spentUsd),
    calls: Math.round(Number(r.calls) || 0),
  };
}

function normalizeInsights(
  raw: unknown,
  fallbackStart: string,
  fallbackEnd: string
): TeamUsageInsights {
  const r = asRecord(raw) ?? {};
  const period = asRecord(r.period) ?? {};
  const totals = asRecord(r.totals) ?? {};
  const dailySeries = Array.isArray(r.dailySeries) ? r.dailySeries.map(normalizeDailyPoint) : [];
  const topModels = Array.isArray(r.topModels) ? r.topModels.map(normalizeModelRow) : [];
  const topIntegrations = Array.isArray(r.topIntegrations)
    ? r.topIntegrations.map(normalizeIntegrationRow)
    : [];
  return {
    period: {
      startDate: typeof period.startDate === 'string' ? period.startDate : fallbackStart,
      endDate: typeof period.endDate === 'string' ? period.endDate : fallbackEnd,
    },
    totals: {
      inferenceUsd: normalizeUsd(totals.inferenceUsd),
      integrationsUsd: normalizeUsd(totals.integrationsUsd),
      totalUsd: normalizeUsd(totals.totalUsd),
      inferenceCalls: Math.round(Number(totals.inferenceCalls) || 0),
      integrationCalls: Math.round(Number(totals.integrationCalls) || 0),
    },
    dailySeries,
    topModels,
    topIntegrations,
  };
}

export function normalizeTeamUsage(payload: unknown): TeamUsage {
  const raw = (
    payload && typeof payload === 'object' && !Array.isArray(payload) ? payload : {}
  ) as Record<string, unknown>;
  const cycleStartDate =
    typeof raw.cycleStartDate === 'string' ? raw.cycleStartDate : new Date().toISOString();
  const cycleEndsAt =
    typeof raw.cycleEndsAt === 'string' ? raw.cycleEndsAt : new Date().toISOString();
  return {
    remainingUsd: normalizeUsd(raw.remainingUsd),
    cycleBudgetUsd: normalizeUsd(raw.cycleBudgetUsd),
    cycleSpentUsd: normalizeUsd(raw.cycleSpentUsd),
    cycleStartDate,
    cycleEndsAt,
    plan: normalizePlanSummary(raw.plan),
    insights: normalizeInsights(raw.insights, cycleStartDate, cycleEndsAt),
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
    const result = await callCoreCommand<TeamUsage>('openhuman.team_get_usage');
    return normalizeTeamUsage(result);
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
