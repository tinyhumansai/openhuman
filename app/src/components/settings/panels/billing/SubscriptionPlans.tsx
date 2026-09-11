import { useT } from '../../../../lib/i18n/I18nContext';
import type { PlanTier } from '../../../../types/api';
import Alert from '../../../ui/Alert';
import Badge from '../../../ui/Badge';
import Button from '../../../ui/Button';
import Card from '../../../ui/Card';
import { CheckIcon, Spinner } from '../../../ui/icons';
import { SettingsSwitch } from '../../controls';
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
  upgradesDisabled?: boolean;
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
  upgradesDisabled = false,
  onUpgrade,
}: SubscriptionPlansProps) => {
  const { t } = useT();
  return (
    <>
      <Card className="p-4">
        <h3 className="font-headline text-2xl font-bold tracking-tight text-content">
          {t('settings.billing.subscription.chooseTitle')}
        </h3>
        <p className="mt-1 text-sm text-content-muted">
          {t('settings.billing.subscription.chooseSubtitle')}
        </p>

        <div className="mt-4 flex items-center justify-between">
          <div>
            <p className="text-sm font-semibold text-content">
              {t('settings.billing.subscription.cryptoQuestion')}
            </p>
            <p className="mt-0.5 text-xs text-content-muted">
              {t('settings.billing.subscription.cryptoDesc')}
            </p>
          </div>
          <SettingsSwitch
            id="subscription-crypto-toggle"
            checked={paymentMethod === 'crypto'}
            onCheckedChange={next => setPaymentMethod(next ? 'crypto' : 'card')}
          />
        </div>
      </Card>

      <div className="flex flex-col gap-4">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-center lg:justify-between">
          <div className="mx-auto inline-flex w-fit rounded-full bg-surface p-1 shadow-xs ring-1 ring-line lg:mx-0">
            <Button
              variant={billingInterval === 'monthly' ? 'primary' : 'tertiary'}
              size="sm"
              className="rounded-full"
              onClick={() => {
                if (paymentMethod !== 'crypto') setBillingInterval('monthly');
              }}
              disabled={paymentMethod === 'crypto'}>
              {t('settings.billing.subscription.monthly')}
            </Button>
            <Button
              variant={billingInterval === 'annual' ? 'primary' : 'tertiary'}
              size="sm"
              className="rounded-full"
              onClick={() => setBillingInterval('annual')}>
              {t('settings.billing.subscription.annual')}
            </Button>
          </div>
        </div>

        {paymentConfirmed && (
          <Alert variant="success">
            <CheckIcon className="h-4 w-4 shrink-0" />
            <p className="text-sm font-medium">
              {t('settings.billing.subscription.paymentConfirmed')}
            </p>
          </Alert>
        )}

        {isPurchasing && (
          <Alert variant="warning">
            <Spinner className="h-4 w-4" />
            <p className="text-sm">{t('settings.billing.subscription.waitingPayment')}</p>
          </Alert>
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
                    ? 'bg-primary-50 dark:bg-primary-500/10 ring-2 ring-primary-500 shadow-xs'
                    : isCurrent
                      ? 'bg-surface ring-1 ring-primary-200 shadow-xs'
                      : 'bg-surface ring-1 ring-line shadow-xs'
                }`}>
                <div className="flex items-start gap-4">
                  <div
                    className={`flex h-12 w-12 min-h-12 min-w-12 shrink-0 items-center justify-center rounded-full ${
                      plan.recommended
                        ? 'bg-primary-600 text-content-inverted'
                        : isCurrent
                          ? 'bg-primary-100 dark:bg-primary-500/20 text-primary-700 dark:text-primary-300'
                          : 'bg-surface-subtle text-content-secondary'
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
                      <h4 className="font-headline text-xl font-bold tracking-tight text-content">
                        {plan.name}
                      </h4>
                      {isPopular && (
                        <Badge
                          variant="primary"
                          className="rounded-full bg-primary-600 px-2.5 py-1 text-[10px] font-bold uppercase tracking-[0.24em] text-content-inverted">
                          {t('settings.billing.subscription.popular')}
                        </Badge>
                      )}
                      {isCurrent && !plan.recommended && (
                        <Badge
                          variant="neutral"
                          className="rounded-full bg-content px-2.5 py-1 text-[10px] font-bold uppercase tracking-[0.24em] text-content-inverted">
                          {t('settings.billing.subscription.current')}
                        </Badge>
                      )}
                    </div>
                    <div className="mt-3 flex flex-wrap gap-2">
                      {plan.features.slice(0, 4).map(feature => (
                        <Badge
                          key={feature.text}
                          variant="neutral"
                          className="rounded-full border-primary-200 bg-surface-subtle/50 px-3 py-1 text-xs font-medium normal-case dark:border-primary-500/30">
                          {feature.text}
                        </Badge>
                      ))}
                    </div>
                  </div>
                </div>

                <div className="flex items-end justify-between gap-2 sm:min-w-[148px] sm:flex-col sm:items-end">
                  <div className="text-right">
                    <p className="text-2xl font-bold tracking-tight text-content">
                      {displayPrice(plan, billingInterval)}
                      {plan.tier !== 'FREE' && (
                        <span className="text-sm font-medium text-content-faint">
                          {t('settings.billing.subscription.perMonth')}
                        </span>
                      )}
                    </p>
                    {plan.tier !== 'FREE' && billingInterval === 'annual' && (
                      <p className="mt-1 text-xs text-content-muted">
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
                    <Badge
                      variant="primary"
                      className="rounded-full bg-primary-600 px-4 py-2 text-xs font-semibold normal-case text-content-inverted">
                      {t('settings.billing.subscription.currentPlan')}
                    </Badge>
                  ) : isUpgrade ? (
                    <Button
                      variant="primary"
                      size="sm"
                      className="rounded-full"
                      onClick={() => onUpgrade(plan.tier)}
                      disabled={isPurchasing || upgradesDisabled}>
                      {isThisPurchasing
                        ? t('settings.billing.subscription.waiting')
                        : t('settings.billing.subscription.upgrade')}
                    </Button>
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
