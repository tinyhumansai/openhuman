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
  fiveHourCapUsd: number;
  marginPercent: number;
  storageLimitBytes: number;
  features: PlanFeature[];
}

export const PLANS: PlanMeta[] = [
  {
    tier: 'FREE',
    name: 'Free',
    monthlyPrice: 0,
    annualPrice: 0,
    monthlyBudgetUsd: 1,
    weeklyBudgetUsd: 0.5,
    fiveHourCapUsd: 0.15,
    marginPercent: 100,
    storageLimitBytes: 100 * 1024 * 1024,
    features: [
      { text: 'Base access to integrations and inference', included: true },
      { text: 'Pay-as-you-go top-ups when included usage runs out', included: true },
      { text: 'Highest internal markup', included: true },
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
    marginPercent: 80,
    storageLimitBytes: 10 * 1024 * 1024 * 1024,
    features: [
      { text: 'Higher included premium usage every billing cycle', included: true },
      { text: 'Lower markup on integrations, bandwidth, and inference', included: true },
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
    marginPercent: 60,
    storageLimitBytes: 200 * 1024 * 1024 * 1024,
    features: [
      { text: 'Largest included premium usage allocation', included: true },
      { text: 'Lowest markup on premium integrations and inference', included: true },
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

export function formatStorageLimit(bytes: number): string {
  const gb = 1024 * 1024 * 1024;
  const mb = 1024 * 1024;

  if (bytes >= gb) return `${Math.round(bytes / gb)} GB`;
  return `${Math.round(bytes / mb)} MB`;
}
