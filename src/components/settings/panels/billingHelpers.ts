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
  features: PlanFeature[];
}

export const PLANS: PlanMeta[] = [
  {
    tier: 'FREE',
    name: 'Free',
    monthlyPrice: 0,
    annualPrice: 0,
    features: [{ text: 'Limited Access', included: true }],
  },
  {
    tier: 'BASIC',
    name: 'Basic',
    monthlyPrice: 25,
    annualPrice: 250,
    features: [{ text: '20x Priority Access', included: true }],
  },
  {
    tier: 'PRO',
    name: 'Pro',
    monthlyPrice: 200,
    annualPrice: 2000,
    features: [{ text: '400x Priority Access', included: true }],
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
