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
    features: [
      { text: 'Base access to integrations and inference', included: true },
      { text: 'One-time signup credits when available', included: true },
      { text: 'Pay-as-you-go top-ups when credits run out', included: true },
      { text: 'No subscription discount on premium usage', included: true },
    ],
  },
  {
    tier: 'BASIC',
    name: 'Basic',
    monthlyPrice: 20,
    annualPrice: 200,
    monthlyBudgetUsd: 20,
    weeklyBudgetUsd: 10,
    fiveHourCapUsd: 3,
    discountPercent: 20,
    features: [
      { text: 'Higher included premium usage every billing cycle', included: true },
      {
        text: '20% premium-usage discount across integrations, bandwidth, and inference',
        included: true,
      },
      { text: 'Pay-as-you-go top-ups for overflow usage', included: true },
    ],
  },
  {
    tier: 'PRO',
    name: 'Pro',
    monthlyPrice: 200,
    annualPrice: 2000,
    monthlyBudgetUsd: 200,
    weeklyBudgetUsd: 100,
    fiveHourCapUsd: 30,
    discountPercent: 40,
    features: [
      { text: 'Largest included premium usage allocation', included: true },
      { text: '40% premium-usage discount across integrations and inference', included: true },
      { text: 'Best fit for heavy bandwidth and agent workloads', included: true },
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
