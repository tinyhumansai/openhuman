import SubscriptionPlans from './SubscriptionPlans';
import type { PlanTier } from '../../../../types/api';

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
        <div>
          <h3 className="font-headline text-2xl font-bold tracking-tight text-stone-950">
            Explore tiers
          </h3>
          <p className="mt-1 text-sm text-stone-500">
            Compare plans, switch billing cadence, and choose card or crypto checkout.
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

      <section>
        <div className="rounded-[28px] border-l-4 border-primary-600 bg-[#eef4ff] p-6 shadow-[0_24px_60px_rgba(15,23,42,0.05)]">
          <div className="flex items-start gap-4">
            <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-white text-primary-600 shadow-sm">
              <svg className="h-6 w-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.8}
                  d="M11.5 4v16m-5-5 5 5 5-5M4 7.5l7.5-3 7.5 3-7.5 3-7.5-3Z"
                />
              </svg>
            </div>
            <div className="space-y-1">
              <h4 className="text-lg font-bold text-stone-950">Crypto payments available</h4>
              <p className="max-w-2xl text-sm leading-6 text-stone-600">
                Secure your subscription using Ethereum, Bitcoin, or USDC. This payment method is
                reserved for annual plan commitments to keep checkout and settlement stable.
              </p>
            </div>
          </div>
        </div>
      </section>
    </>
  );
}
