import { describe, expect, it } from 'vitest';

import type { PlanTier } from '../../../../types/api';
import {
  annualSavings,
  buildPlanId,
  displayPrice,
  isUpgrade,
  type PlanMeta,
  PLANS,
  tierIndex,
} from '../billingHelpers';

describe('PLANS', () => {
  it('should contain exactly 3 plans', () => {
    expect(PLANS).toHaveLength(3);
  });

  it('should have plans in order: FREE, BASIC, PRO', () => {
    expect(PLANS[0].tier).toBe('FREE');
    expect(PLANS[1].tier).toBe('BASIC');
    expect(PLANS[2].tier).toBe('PRO');
  });

  it('should have FREE plan at $0', () => {
    const free = PLANS.find(p => p.tier === 'FREE')!;
    expect(free.monthlyPrice).toBe(0);
    expect(free.annualPrice).toBe(0);
    expect(free.discountPercent).toBe(0);
    expect(free.weeklyBudgetUsd).toBe(0.5);
  });

  it('should have BASIC plan aligned with backend config', () => {
    const basic = PLANS.find(p => p.tier === 'BASIC')!;
    expect(basic.monthlyPrice).toBe(20);
    expect(basic.annualPrice).toBe(200);
    expect(basic.discountPercent).toBe(20);
    expect(basic.monthlyBudgetUsd).toBe(20);
    expect(basic.weeklyBudgetUsd).toBe(10);
    expect(basic.fiveHourCapUsd).toBe(3);
  });

  it('should have PRO plan aligned with backend config', () => {
    const pro = PLANS.find(p => p.tier === 'PRO')!;
    expect(pro.monthlyPrice).toBe(200);
    expect(pro.annualPrice).toBe(2000);
    expect(pro.discountPercent).toBe(40);
    expect(pro.monthlyBudgetUsd).toBe(200);
    expect(pro.weeklyBudgetUsd).toBe(100);
    expect(pro.fiveHourCapUsd).toBe(30);
  });

  it('should have features for every plan', () => {
    for (const plan of PLANS) {
      expect(plan.features.length).toBeGreaterThan(0);
    }
  });
});

describe('tierIndex', () => {
  it('should return 0 for FREE', () => {
    expect(tierIndex('FREE')).toBe(0);
  });

  it('should return 1 for BASIC', () => {
    expect(tierIndex('BASIC')).toBe(1);
  });

  it('should return 2 for PRO', () => {
    expect(tierIndex('PRO')).toBe(2);
  });

  it('should return -1 for unknown tier', () => {
    expect(tierIndex('UNKNOWN' as PlanTier)).toBe(-1);
  });
});

describe('buildPlanId', () => {
  it('should build BASIC_MONTHLY', () => {
    expect(buildPlanId('BASIC', 'monthly')).toBe('BASIC_MONTHLY');
  });

  it('should build BASIC_YEARLY', () => {
    expect(buildPlanId('BASIC', 'annual')).toBe('BASIC_YEARLY');
  });

  it('should build PRO_MONTHLY', () => {
    expect(buildPlanId('PRO', 'monthly')).toBe('PRO_MONTHLY');
  });

  it('should build PRO_YEARLY', () => {
    expect(buildPlanId('PRO', 'annual')).toBe('PRO_YEARLY');
  });

  it('should build FREE_MONTHLY (even though not used in practice)', () => {
    expect(buildPlanId('FREE', 'monthly')).toBe('FREE_MONTHLY');
  });
});

describe('displayPrice', () => {
  const basicPlan = PLANS.find(p => p.tier === 'BASIC')!;
  const proPlan = PLANS.find(p => p.tier === 'PRO')!;
  const freePlan = PLANS.find(p => p.tier === 'FREE')!;

  describe('monthly billing', () => {
    it('should return $0 for FREE plan', () => {
      expect(displayPrice(freePlan, 'monthly')).toBe('$0');
    });

    it('should return $20 for BASIC plan', () => {
      expect(displayPrice(basicPlan, 'monthly')).toBe('$20');
    });

    it('should return $200 for PRO plan', () => {
      expect(displayPrice(proPlan, 'monthly')).toBe('$200');
    });
  });

  describe('annual billing', () => {
    it('should return $0 for FREE plan', () => {
      expect(displayPrice(freePlan, 'annual')).toBe('$0');
    });

    it('should return annual equivalent monthly price for BASIC ($200/12 = $17)', () => {
      expect(displayPrice(basicPlan, 'annual')).toBe('$17');
    });

    it('should return annual equivalent monthly price for PRO ($2000/12 = $167)', () => {
      // $2000 / 12 = 166.67, rounded to $167
      expect(displayPrice(proPlan, 'annual')).toBe('$167');
    });
  });

  it('should handle a custom plan correctly', () => {
    const custom: PlanMeta = {
      tier: 'BASIC',
      name: 'Custom',
      monthlyPrice: 50,
      annualPrice: 480,
      monthlyBudgetUsd: 50,
      weeklyBudgetUsd: 25,
      fiveHourCapUsd: 7.5,
      discountPercent: 30,
      storageLimitBytes: 50 * 1024 * 1024 * 1024,
      features: [],
    };
    expect(displayPrice(custom, 'monthly')).toBe('$50');
    // $480 / 12 = $40
    expect(displayPrice(custom, 'annual')).toBe('$40');
  });
});

describe('annualSavings', () => {
  const basicPlan = PLANS.find(p => p.tier === 'BASIC')!;
  const proPlan = PLANS.find(p => p.tier === 'PRO')!;
  const freePlan = PLANS.find(p => p.tier === 'FREE')!;

  it('should return null for FREE plan regardless of interval', () => {
    expect(annualSavings(freePlan, 'annual')).toBeNull();
    expect(annualSavings(freePlan, 'monthly')).toBeNull();
  });

  it('should return null for monthly billing interval', () => {
    expect(annualSavings(basicPlan, 'monthly')).toBeNull();
    expect(annualSavings(proPlan, 'monthly')).toBeNull();
  });

  it('should calculate savings for BASIC annual', () => {
    // Monthly total: $20 * 12 = $240, Annual: $200
    // Savings: ($240 - $200) / $240 = 16.67%, rounded to 17%
    expect(annualSavings(basicPlan, 'annual')).toBe(17);
  });

  it('should calculate savings for PRO annual', () => {
    // Monthly total: $200 * 12 = $2400, Annual: $2000
    // Savings: ($2400 - $2000) / $2400 = 16.67%, rounded to 17%
    expect(annualSavings(proPlan, 'annual')).toBe(17);
  });

  it('should return null when annual price equals monthly * 12 (no savings)', () => {
    const noSavings: PlanMeta = {
      tier: 'BASIC',
      name: 'No Savings',
      monthlyPrice: 10,
      annualPrice: 120, // 10 * 12, no discount
      monthlyBudgetUsd: 10,
      weeklyBudgetUsd: 5,
      fiveHourCapUsd: 1.5,
      discountPercent: 20,
      storageLimitBytes: 1024,
      features: [],
    };
    expect(annualSavings(noSavings, 'annual')).toBeNull();
  });

  it('should return correct percentage for large discount', () => {
    const bigDiscount: PlanMeta = {
      tier: 'PRO',
      name: 'Big Discount',
      monthlyPrice: 100,
      annualPrice: 600, // 50% off
      monthlyBudgetUsd: 100,
      weeklyBudgetUsd: 50,
      fiveHourCapUsd: 15,
      discountPercent: 40,
      storageLimitBytes: 1024,
      features: [],
    };
    expect(annualSavings(bigDiscount, 'annual')).toBe(50);
  });
});

describe('isUpgrade', () => {
  it('should return true when upgrading from FREE to BASIC', () => {
    expect(isUpgrade('BASIC', 'FREE')).toBe(true);
  });

  it('should return true when upgrading from FREE to PRO', () => {
    expect(isUpgrade('PRO', 'FREE')).toBe(true);
  });

  it('should return true when upgrading from BASIC to PRO', () => {
    expect(isUpgrade('PRO', 'BASIC')).toBe(true);
  });

  it('should return false for same tier', () => {
    expect(isUpgrade('FREE', 'FREE')).toBe(false);
    expect(isUpgrade('BASIC', 'BASIC')).toBe(false);
    expect(isUpgrade('PRO', 'PRO')).toBe(false);
  });

  it('should return false when downgrading from PRO to BASIC', () => {
    expect(isUpgrade('BASIC', 'PRO')).toBe(false);
  });

  it('should return false when downgrading from PRO to FREE', () => {
    expect(isUpgrade('FREE', 'PRO')).toBe(false);
  });

  it('should return false when downgrading from BASIC to FREE', () => {
    expect(isUpgrade('FREE', 'BASIC')).toBe(false);
  });
});
