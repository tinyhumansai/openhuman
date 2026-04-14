import type { PlanTier } from '../../../../types/api';
import SubscriptionPlans from './SubscriptionPlans';

interface BillingPlansTabProps {
  billingInterval: 'monthly' | 'annual';
  currentTier: PlanTier;
  isPurchasing: boolean;
  onUpgrade: (tier: PlanTier) => void;
  paymentConfirmed: boolean;
  paymentMethod: 'card' | 'crypto';
  purchasingTier: PlanTier | null;
  setBillingInterval: (value: 'monthly' | 'annual') => void;
  setPaymentMethod: (value: 'card' | 'crypto') => void;
}

export default function BillingPlansTab({
  billingInterval,
  currentTier,
  isPurchasing,
  onUpgrade,
  paymentConfirmed,
  paymentMethod,
  purchasingTier,
  setBillingInterval,
  setPaymentMethod,
}: BillingPlansTabProps) {
  return (
    <SubscriptionPlans
      currentTier={currentTier}
      billingInterval={billingInterval}
      setBillingInterval={setBillingInterval}
      paymentMethod={paymentMethod}
      setPaymentMethod={setPaymentMethod}
      isPurchasing={isPurchasing}
      purchasingTier={purchasingTier}
      paymentConfirmed={paymentConfirmed}
      onUpgrade={onUpgrade}
    />
  );
}
