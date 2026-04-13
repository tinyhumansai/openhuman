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
    <>
      <section className="space-y-4">
        <div className="flex flex-col gap-2 rounded-2xl bg-white p-4 border border-stone-200">
          <h3 className="font-headline text-2xl font-bold tracking-tight text-stone-950">
            Choose a Subscription Plan
          </h3>
          <p className="mt-1 text-sm text-stone-500">
            Compare plans, switch billing cadence, and choose a payment method.
          </p>
        </div>
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
      </section>
    </>
  );
}
