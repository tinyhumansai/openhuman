import { useT } from '../../../../lib/i18n/I18nContext';
import type { PlanTier } from '../../../../types/api';
import { Spinner } from '../../../ui';
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
}: SubscriptionPlansProps) => {
  const { t } = useT();
  return (
    <>
      <div className="flex flex-col gap-2 rounded-2xl bg-white dark:bg-neutral-900 p-4 border border-stone-200 dark:border-neutral-800">
        <h3 className="font-headline text-2xl font-bold tracking-tight text-stone-950 dark:text-neutral-50">
          {t('settings.billing.subscription.chooseTitle')}
        </h3>
        <p className="mt-1 text-sm text-stone-500 dark:text-neutral-400">
          {t('settings.billing.subscription.chooseSubtitle')}
        </p>

        <div className="flex items-center justify-between mt-4">
          <div>
            <p className="text-sm font-semibold text-stone-950 dark:text-neutral-50">
              {t('settings.billing.subscription.cryptoQuestion')}
            </p>
            <p className="mt-0.5 text-xs text-stone-500 dark:text-neutral-400">
              {t('settings.billing.subscription.cryptoDesc')}
            </p>
          </div>
          <button
            onClick={() => setPaymentMethod(paymentMethod === 'card' ? 'crypto' : 'card')}
            className={`relative h-6 w-11 rounded-full transition-colors ${
              paymentMethod === 'crypto' ? 'bg-primary-600' : 'bg-stone-300 dark:bg-neutral-700'
            }`}
            role="switch"
            aria-checked={paymentMethod === 'crypto'}>
            <span
              className={`absolute top-0.5 left-0.5 h-5 w-5 rounded-full bg-white dark:bg-neutral-900 shadow transition-transform ${
                paymentMethod === 'crypto' ? 'translate-x-5' : 'translate-x-0'
              }`}
            />
          </button>
        </div>
      </div>

      <div className="flex flex-col gap-4">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-center lg:justify-between">
          <div className="mx-auto inline-flex w-fit rounded-full bg-white dark:bg-neutral-900 p-1 shadow-sm ring-1 ring-stone-950/5 lg:mx-0">
            <button
              onClick={() => {
                if (paymentMethod !== 'crypto') setBillingInterval('monthly');
              }}
              disabled={paymentMethod === 'crypto'}
              className={`rounded-full px-4 py-2 text-sm font-semibold transition-colors ${
                billingInterval === 'monthly'
                  ? 'bg-primary-600 text-white'
                  : 'text-stone-500 dark:text-neutral-400 hover:text-stone-900 dark:hover:text-neutral-100 dark:text-neutral-100'
              } ${paymentMethod === 'crypto' ? 'cursor-not-allowed opacity-40' : ''}`}>
              {t('settings.billing.subscription.monthly')}
            </button>
            <button
              onClick={() => setBillingInterval('annual')}
              className={`rounded-full px-4 py-2 text-sm font-semibold transition-colors ${
                billingInterval === 'annual'
                  ? 'bg-primary-600 text-white'
                  : 'text-stone-500 dark:text-neutral-400 hover:text-stone-900 dark:hover:text-neutral-100 dark:text-neutral-100'
              }`}>
              {t('settings.billing.subscription.annual')}
            </button>
          </div>
        </div>

        {paymentConfirmed && (
          <div className="rounded-2xl border border-sage-500/20 bg-sage-500/10 p-4">
            <div className="flex items-center gap-2">
              <svg
                className="h-4 w-4 flex-shrink-0 text-sage-500"
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
              <p className="text-sm font-medium text-sage-700 dark:text-sage-300">
                {t('settings.billing.subscription.paymentConfirmed')}
              </p>
            </div>
          </div>
        )}

        {isPurchasing && (
          <div className="rounded-2xl border border-amber-500/20 bg-amber-100/90 p-4">
            <div className="flex items-center gap-2">
              <Spinner className="h-4 w-4 text-amber-500" />
              <p className="text-sm text-amber-700 dark:text-amber-300">
                {t('settings.billing.subscription.waitingPayment')}
              </p>
            </div>
          </div>
        )}

        <div className="space-y-3">
          {PLANS.map(plan => {
            const isCurrent = plan.tier === currentTier;
            const isUpgrade = checkIsUpgrade(plan.tier, currentTier);
            const savings = annualSavings(plan, billingInterval);
            const isThisPurchasing = isPurchasing && purchasingTier === plan.tier;
            const isPopular = plan.recommended && billingInterval === 'annual';

            return (
              <div
                key={plan.tier}
                className={`relative flex flex-col gap-5 rounded-[24px] px-5 py-5 transition-all sm:flex-row sm:items-center sm:justify-between ${
                  isPopular
                    ? 'bg-primary-50 dark:bg-primary-500/10 ring-2 ring-primary-500 shadow-sm'
                    : isCurrent
                      ? 'bg-white dark:bg-neutral-900 ring-1 ring-primary-200 shadow-sm'
                      : 'bg-white dark:bg-neutral-900 ring-1 ring-stone-950/5 shadow-sm'
                }`}>
                <div className="flex items-start gap-4">
                  <div
                    className={`flex h-12 w-12 min-h-12 min-w-12 flex-shrink-0 items-center justify-center rounded-full ${
                      plan.recommended
                        ? 'bg-primary-600 text-white'
                        : isCurrent
                          ? 'bg-primary-100 dark:bg-primary-500/20 text-primary-700 dark:text-primary-300'
                          : 'bg-stone-100 dark:bg-neutral-800 text-stone-700 dark:text-neutral-200'
                    }`}>
                    {plan.tier === 'PRO' ? (
                      <svg className="h-5 w-5" fill="currentColor" viewBox="0 0 24 24">
                        <path d="M12 2 9.2 8.5 2 9.2l5.4 4.7-1.6 7.1L12 17l6.2 4-1.6-7.1L22 9.2l-7.2-.7z" />
                      </svg>
                    ) : plan.tier === 'BASIC' ? (
                      <svg
                        className="h-5 w-5"
                        fill="none"
                        stroke="currentColor"
                        viewBox="0 0 24 24">
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={2}
                          d="M5 12h14M12 5l7 7-7 7"
                        />
                      </svg>
                    ) : (
                      <svg
                        className="h-5 w-5"
                        fill="none"
                        stroke="currentColor"
                        viewBox="0 0 24 24">
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          strokeWidth={2}
                          d="M12 12c2.761 0 5-2.239 5-5S14.761 2 12 2 7 4.239 7 7s2.239 5 5 5Zm0 2c-4.418 0-8 1.79-8 4v2h16v-2c0-2.21-3.582-4-8-4Z"
                        />
                      </svg>
                    )}
                  </div>

                  <div>
                    <div className="flex flex-wrap items-center gap-2">
                      <h4 className="font-headline text-xl font-bold tracking-tight text-stone-950 dark:text-neutral-50">
                        {plan.name}
                      </h4>
                      {isPopular && (
                        <span className="rounded-full bg-primary-600 px-2.5 py-1 text-[10px] font-bold uppercase tracking-[0.24em] text-white">
                          {t('settings.billing.subscription.popular')}
                        </span>
                      )}
                      {isCurrent && !plan.recommended && (
                        <span className="rounded-full bg-stone-950 dark:bg-neutral-50 px-2.5 py-1 text-[10px] font-bold uppercase tracking-[0.24em] text-white">
                          {t('settings.billing.subscription.current')}
                        </span>
                      )}
                    </div>
                    <div className="mt-3 flex flex-wrap gap-2">
                      {plan.features.slice(0, 4).map(feature => (
                        <span
                          key={feature.text}
                          className="rounded-full bg-stone-100/50 border border-primary-200 dark:border-primary-500/30 px-3 py-1 text-xs font-medium text-stone-600 dark:text-neutral-300">
                          {feature.text}
                        </span>
                      ))}
                    </div>
                  </div>
                </div>

                <div className="flex items-end justify-between gap-2 sm:min-w-[148px] sm:flex-col sm:items-end">
                  <div className="text-right">
                    <p className="text-2xl font-bold tracking-tight text-stone-950 dark:text-neutral-50">
                      {displayPrice(plan, billingInterval)}
                      {plan.tier !== 'FREE' && (
                        <span className="text-sm font-medium text-stone-400 dark:text-neutral-500">
                          {t('settings.billing.subscription.perMonth')}
                        </span>
                      )}
                    </p>
                    {plan.tier !== 'FREE' && billingInterval === 'annual' && (
                      <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
                        {t('settings.billing.subscription.billedAnnually').replace(
                          '{price}',
                          String(plan.annualPrice)
                        )}
                      </p>
                    )}
                    {savings && (
                      <p className="mt-1 text-xs font-semibold uppercase text-primary-600 dark:text-primary-300">
                        {t('settings.billing.subscription.save').replace('{pct}', String(savings))}
                      </p>
                    )}
                  </div>

                  {isCurrent ? (
                    <div className="rounded-full bg-primary-600 px-4 py-2 text-xs font-semibold text-white">
                      {t('settings.billing.subscription.currentPlan')}
                    </div>
                  ) : isUpgrade ? (
                    <button
                      onClick={() => onUpgrade(plan.tier)}
                      disabled={isPurchasing}
                      className={`rounded-full px-4 py-2 text-xs font-semibold transition-colors ${
                        isPurchasing
                          ? 'cursor-not-allowed bg-stone-200 dark:bg-neutral-800 text-stone-400 dark:text-neutral-500'
                          : 'bg-stone-950 dark:bg-neutral-50 text-white hover:bg-primary-600'
                      }`}>
                      {isThisPurchasing
                        ? t('settings.billing.subscription.waiting')
                        : t('settings.billing.subscription.upgrade')}
                    </button>
                  ) : null}
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </>
  );
};

export default SubscriptionPlans;
