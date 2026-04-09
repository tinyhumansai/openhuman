import type { PlanTier } from '../../../../types/api';
import {
  annualSavings,
  isUpgrade as checkIsUpgrade,
  displayPrice,
  formatStorageLimit,
  formatUsdAmount,
  PLANS,
} from '../billingHelpers';

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
    <div className="flex items-center justify-center gap-2 px-4">
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
    <div className="space-y-2 px-4">
      {PLANS.map(plan => {
        const isCurrent = plan.tier === currentTier;
        const isUpgrade = checkIsUpgrade(plan.tier, currentTier);
        const savings = annualSavings(plan, billingInterval);
        const isThisPurchasing = isPurchasing && purchasingTier === plan.tier;

        return (
          <div
            key={plan.tier}
            className={`rounded-2xl border p-3 transition-all ${
              isCurrent ? 'border-primary-500/40 bg-primary-500/5' : 'border-stone-200 bg-white'
            }`}>
            <div className="flex items-start justify-between mb-2">
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2 flex-wrap">
                  <h4 className="text-sm font-semibold text-stone-900">{plan.name}</h4>
                  {plan.features.map(f => (
                    <span key={f.text} className="text-xs text-stone-600">
                      <span className="text-stone-500 mx-1">&bull;</span>
                      {f.text}
                    </span>
                  ))}
                  {isCurrent && (
                    <span className="px-1.5 py-0.5 text-[10px] font-medium rounded-full bg-primary-500/20 text-primary-400 border border-primary-500/30">
                      Current
                    </span>
                  )}
                  {savings && (
                    <span className="px-1.5 py-0.5 text-[10px] font-medium rounded-full bg-sage-500/20 text-sage-400 border border-sage-500/30">
                      Save {savings}%
                    </span>
                  )}
                </div>
                <div className="mt-0.5 flex items-baseline gap-1">
                  <span className="text-xl font-bold text-stone-900">
                    {displayPrice(plan, billingInterval)}
                  </span>
                  {plan.tier !== 'FREE' && <span className="text-xs text-stone-400">/mo</span>}
                  {plan.tier !== 'FREE' && billingInterval === 'annual' && (
                    <span className="text-xs text-stone-500 ml-1">
                      (billed ${plan.annualPrice}/yr)
                    </span>
                  )}
                </div>
                <div className="mt-2 flex flex-wrap gap-1.5">
                  <span className="rounded-full border border-stone-200 bg-stone-50 px-2 py-1 text-[10px] text-stone-600">
                    Included monthly value: {formatUsdAmount(plan.monthlyBudgetUsd)}
                  </span>
                  <span className="rounded-full border border-stone-200 bg-stone-50 px-2 py-1 text-[10px] text-stone-600">
                    7-day cycle: {formatUsdAmount(plan.weeklyBudgetUsd)}
                  </span>
                  <span className="rounded-full border border-stone-200 bg-stone-50 px-2 py-1 text-[10px] text-stone-600">
                    10-hour cap: {formatUsdAmount(plan.fiveHourCapUsd)}
                  </span>
                  <span className="rounded-full border border-stone-200 bg-stone-50 px-2 py-1 text-[10px] text-stone-600">
                    Discount: {plan.discountPercent}%
                  </span>
                  <span className="rounded-full border border-stone-200 bg-stone-50 px-2 py-1 text-[10px] text-stone-600">
                    Storage: {formatStorageLimit(plan.storageLimitBytes)}
                  </span>
                </div>
              </div>

              {/* Action button */}
              {isUpgrade && (
                <button
                  onClick={() => onUpgrade(plan.tier)}
                  disabled={isPurchasing}
                  className={`px-3 py-1.5 text-xs font-medium rounded-lg transition-colors flex-shrink-0 ${
                    isPurchasing
                      ? 'bg-stone-700/40 text-stone-500 cursor-not-allowed'
                      : 'bg-primary-500 hover:bg-primary-600 text-white'
                  }`}>
                  {isThisPurchasing ? 'Waiting...' : 'Upgrade'}
                </button>
              )}
            </div>
          </div>
        );
      })}
    </div>

    {/* Payment confirmed banner */}
    {paymentConfirmed && (
      <div className="rounded-xl bg-sage-500/10 border border-sage-500/20 p-3 mx-4">
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
      <div className="rounded-xl bg-amber-500/10 border border-amber-500/20 p-3 mx-4">
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
    <div className="flex items-center justify-between rounded-xl bg-stone-50 border border-stone-200 p-3 mx-4">
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
