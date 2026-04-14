import type { PlanIdentifier, PlanTier } from '../../../types/api';

export interface PlanFeature {
  text: string;
  included: boolean;
}

export interface PlanMeta {
  tier: PlanTier;
  name: string;
  monthlyPrice: number;
  annualPrice: number;
  monthlyBudgetUsd: number;
  weeklyBudgetUsd: number;
  /** USD cap per 10-hour rolling inference window; amount scales with `tier` (FREE / BASIC / PRO). */
  fiveHourCapUsd: number;
  discountPercent: number;
  features: PlanFeature[];
  recommended?: boolean;
  tagline?: string;
}

export const PLANS: PlanMeta[] = [
  {
    tier: 'FREE',
    name: 'Free',
    monthlyPrice: 0,
    annualPrice: 0,
    monthlyBudgetUsd: 0,
    weeklyBudgetUsd: 0,
    fiveHourCapUsd: 0,
    discountPercent: 0,
    tagline: 'Get started at no cost',
    features: [
      { text: 'Access to Everything', included: true },
      { text: 'Heavy Rate Limits', included: true },
      { text: 'Pay-as-you-go', included: true },
      { text: 'No discounts', included: false },
    ],
  },
  {
    tier: 'BASIC',
    name: 'Basic',
    monthlyPrice: 19.99,
    annualPrice: 199,
    monthlyBudgetUsd: 20,
    weeklyBudgetUsd: 10,
    fiveHourCapUsd: 3,
    discountPercent: 20,
    recommended: true,
    tagline: 'Best value for most users',
    features: [
      { text: 'Everything in Free', included: true },
      { text: '20x more usage', included: true },
      { text: 'Cloud features enabled', included: true },
    ],
  },
  {
    tier: 'PRO',
    name: 'Pro',
    monthlyPrice: 199.99,
    annualPrice: 1799.99,
    monthlyBudgetUsd: 199,
    weeklyBudgetUsd: 99,
    fiveHourCapUsd: 30,
    discountPercent: 40,
    tagline: 'For power users and teams',
    features: [
      { text: 'Everything in Basic', included: true },
      { text: '40x more usage', included: true },
      { text: 'Higher Rate Limits', included: true },
    ],
  },
];

export function tierIndex(tier: PlanTier): number {
  return PLANS.findIndex(p => p.tier === tier);
}

export function buildPlanId(tier: PlanTier, interval: 'monthly' | 'annual'): PlanIdentifier {
  const suffix = interval === 'annual' ? 'YEARLY' : 'MONTHLY';
  return `${tier}_${suffix}` as PlanIdentifier;
}

export function displayPrice(plan: PlanMeta, billingInterval: 'monthly' | 'annual'): string {
  if (plan.tier === 'FREE') return '$0';
  if (billingInterval === 'annual') {
    const monthly = Math.round(plan.annualPrice / 12);
    return `$${monthly}`;
  }
  return `$${plan.monthlyPrice}`;
}

export function annualSavings(
  plan: PlanMeta,
  billingInterval: 'monthly' | 'annual'
): number | null {
  if (plan.tier === 'FREE' || billingInterval !== 'annual') return null;
  const monthlyTotal = plan.monthlyPrice * 12;
  const pct = Math.round(((monthlyTotal - plan.annualPrice) / monthlyTotal) * 100);
  return pct > 0 ? pct : null;
}

export function isUpgrade(targetTier: PlanTier, currentTier: PlanTier): boolean {
  return tierIndex(targetTier) > tierIndex(currentTier);
}

export function getPlanMeta(tier: PlanTier): PlanMeta | undefined {
  return PLANS.find(plan => plan.tier === tier);
}

export function formatUsdAmount(amount: number): string {
  if (Number.isInteger(amount)) return `$${amount}`;
  return `$${amount.toFixed(2).replace(/0+$/, '').replace(/\.$/, '')}`;
}
