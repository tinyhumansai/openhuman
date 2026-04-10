import type { PlanTier } from '../../../../types/api';
import { annualSavings, isUpgrade as checkIsUpgrade, displayPrice, PLANS } from '../billingHelpers';

interface SubscriptionPlansProps {
  currentTier: PlanTier;
  billingInterval: 'monthly' | 'annual';
  setBillingInterval: (v: 'monthly' | 'annual') => void;
  paymentMethod: 'card' | 'crypto';
  setPaymentMethod: (v: 'card' | 'crypto') => void;
  isPurchasing: boolean;
  purchasingTier: PlanTier | null;
  paymentConfirmed: boolean;
  onUpgrade: (tier: PlanTier) => void;
}

const SubscriptionPlans = ({
  currentTier,
  billingInterval,
  setBillingInterval,
  paymentMethod,
  setPaymentMethod,
  isPurchasing,
  purchasingTier,
  paymentConfirmed,
  onUpgrade,
}: SubscriptionPlansProps) => (
  <>
    {/* Interval toggle */}
    <div className="flex items-center justify-center gap-2">
      <button
        onClick={() => {
          if (paymentMethod !== 'crypto') setBillingInterval('monthly');
        }}
        disabled={paymentMethod === 'crypto'}
        className={`px-3 py-1.5 text-xs font-medium rounded-lg transition-colors ${
          billingInterval === 'monthly'
            ? 'bg-primary-500/20 text-primary-400 border border-primary-500/30'
            : 'text-stone-500 hover:text-stone-700'
        } ${paymentMethod === 'crypto' ? 'opacity-40 cursor-not-allowed' : ''}`}>
        Monthly
      </button>
      <button
        onClick={() => setBillingInterval('annual')}
        className={`px-3 py-1.5 text-xs font-medium rounded-lg transition-colors ${
          billingInterval === 'annual'
            ? 'bg-primary-500/20 text-primary-400 border border-primary-500/30'
            : 'text-stone-500 hover:text-stone-700'
        }`}>
        Annual
      </button>
    </div>

    {/* Plan tier cards */}
    <div className="space-y-3">
      {PLANS.map(plan => {
        const isCurrent = plan.tier === currentTier;
        const isUpgrade = checkIsUpgrade(plan.tier, currentTier);
        const savings = annualSavings(plan, billingInterval);
        const isThisPurchasing = isPurchasing && purchasingTier === plan.tier;

        return (
          <div
            key={plan.tier}
            className={`relative rounded-2xl border p-4 transition-all ${
              plan.recommended
                ? 'border-primary-500 bg-primary-500/5 shadow-sm'
                : isCurrent
                  ? 'border-primary-500/40 bg-primary-500/5'
                  : 'border-stone-200 bg-white'
            }`}>
            {/* Popular badge */}
            {plan.recommended && (
              <span className="absolute -top-2.5 left-4 px-2.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide rounded-full bg-primary-500 text-white">
                Popular
              </span>
            )}

            {/* Header: name + tagline on left, price on right */}
            <div className="flex items-start justify-between">
              <div>
                <h4 className="text-sm font-bold text-stone-900">{plan.name}</h4>
                {plan.tagline && <p className="text-xs text-stone-400 mt-0.5">{plan.tagline}</p>}
              </div>
              <div className="text-right flex-shrink-0">
                <div className="flex items-baseline gap-0.5 justify-end">
                  <span className="text-2xl font-bold text-stone-900">
                    {displayPrice(plan, billingInterval)}
                  </span>
                  {plan.tier !== 'FREE' && <span className="text-xs text-stone-400">/mo</span>}
                </div>
                {plan.tier !== 'FREE' && billingInterval === 'annual' && (
                  <p className="text-[11px] text-stone-400 mt-0.5">billed ${plan.annualPrice}/yr</p>
                )}
                {savings && (
                  <span className="inline-block mt-1 px-2 py-0.5 text-[10px] font-medium rounded-full bg-sage-500/20 text-sage-500 border border-sage-500/30">
                    Save {savings}%
                  </span>
                )}
              </div>
            </div>

            {/* Divider */}
            <div className="h-px bg-stone-100 my-3" />

            {/* Feature list */}
            <ul className="space-y-2">
              {plan.features.map(f => (
                <li key={f.text} className="flex items-start gap-2">
                  {f.included ? (
                    <svg
                      className="w-4 h-4 text-sage-500 flex-shrink-0 mt-0.5"
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M5 13l4 4L19 7"
                      />
                    </svg>
                  ) : (
                    <svg
                      className="w-4 h-4 text-stone-300 flex-shrink-0 mt-0.5"
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M6 18L18 6M6 6l12 12"
                      />
                    </svg>
                  )}
                  <span className={`text-xs ${f.included ? 'text-stone-600' : 'text-stone-400'}`}>
                    {f.text}
                  </span>
                </li>
              ))}
            </ul>

            {/* CTA */}
            <div className="mt-4">
              {isCurrent ? (
                <div className="w-full py-2 text-center text-xs font-medium rounded-lg border border-primary-500/30 bg-primary-500/10 text-primary-500">
                  Current Plan
                </div>
              ) : isUpgrade ? (
                <button
                  onClick={() => onUpgrade(plan.tier)}
                  disabled={isPurchasing}
                  className={`w-full py-2 text-xs font-medium rounded-lg transition-colors ${
                    isPurchasing
                      ? 'bg-stone-200 text-stone-400 cursor-not-allowed'
                      : plan.recommended
                        ? 'bg-primary-500 hover:bg-primary-600 text-white'
                        : 'bg-stone-900 hover:bg-stone-800 text-white'
                  }`}>
                  {isThisPurchasing ? 'Waiting...' : 'Upgrade'}
                </button>
              ) : null}
            </div>
          </div>
        );
      })}
    </div>

    {/* Payment confirmed banner */}
    {paymentConfirmed && (
      <div className="rounded-xl bg-sage-500/10 border border-sage-500/20 p-3">
        <div className="flex items-center gap-2">
          <svg
            className="w-4 h-4 text-sage-400 flex-shrink-0"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
          </svg>
          <p className="text-xs text-sage-300 font-medium">
            Payment confirmed! Your plan has been updated.
          </p>
        </div>
      </div>
    )}

    {/* Purchasing overlay message */}
    {isPurchasing && (
      <div className="rounded-xl bg-amber-500/10 border border-amber-500/20 p-3">
        <div className="flex items-center gap-2">
          <svg className="w-4 h-4 text-amber-400 animate-spin" fill="none" viewBox="0 0 24 24">
            <circle
              className="opacity-25"
              cx="12"
              cy="12"
              r="10"
              stroke="currentColor"
              strokeWidth="4"
            />
            <path
              className="opacity-75"
              fill="currentColor"
              d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
            />
          </svg>
          <p className="text-xs text-amber-700">
            Waiting for payment confirmation... Complete checkout in the browser window that opened.
          </p>
        </div>
      </div>
    )}

    {/* Pay with crypto toggle */}
    <div className="flex items-center justify-between rounded-xl bg-stone-50 border border-stone-200 p-3">
      <div>
        <p className="text-xs font-medium text-stone-900">Pay with Crypto</p>
        <p className="text-[11px] text-stone-400 mt-0.5">
          You can choose to pay annually using crypto
        </p>
      </div>
      <button
        onClick={() => setPaymentMethod(paymentMethod === 'card' ? 'crypto' : 'card')}
        className={`relative w-10 h-5 rounded-full transition-colors ${
          paymentMethod === 'crypto' ? 'bg-primary-500' : 'bg-stone-600'
        }`}
        role="switch"
        aria-checked={paymentMethod === 'crypto'}>
        <span
          className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform ${
            paymentMethod === 'crypto' ? 'translate-x-5' : 'translate-x-0'
          }`}
        />
      </button>
    </div>
  </>
);

export default SubscriptionPlans;
