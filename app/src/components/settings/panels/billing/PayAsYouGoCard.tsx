import { useState } from 'react';

import { useT } from '../../../../lib/i18n/I18nContext';
import { type CreditBalance } from '../../../../services/api/creditsApi';
import Button from '../../../ui/Button';
import { SettingsTextField } from '../../controls';

interface PayAsYouGoCardProps {
  creditBalance: CreditBalance | null;
  isLoadingCredits: boolean;
  isToppingUp: boolean;
  onTopUp: (amountUsd: number) => void;
}

const PayAsYouGoCard = ({
  creditBalance,
  isLoadingCredits,
  isToppingUp,
  onTopUp,
}: PayAsYouGoCardProps) => {
  const { t } = useT();
  const promoCredits = creditBalance?.promotionBalanceUsd ?? 0;
  const teamTopupCredits = creditBalance?.teamTopupUsd ?? 0;
  const availableCredits = promoCredits + teamTopupCredits;

  const [customTopUpAmount, setCustomTopUpAmount] = useState('');
  const customTopUpAmountValid = Number(customTopUpAmount) > 0;

  const handleCustomTopUp = () => {
    if (!customTopUpAmountValid || isToppingUp) return;
    onTopUp(Number(customTopUpAmount));
  };

  return (
    <>
      <div className="rounded-lg bg-surface p-6 shadow-[0_24px_70px_rgba(15,23,42,0.06)] ring-1 ring-neutral-950/5">
        <h3 className="font-headline text-xl font-bold tracking-tight text-content">
          {t('settings.billing.payAsYouGo.creditBalanceTitle')}
        </h3>
        <p className="mt-1 text-sm text-content-muted">
          {t('settings.billing.payAsYouGo.creditBalanceDesc')}
        </p>
        {creditBalance ? (
          <div className="grid mt-4 gap-3 sm:grid-cols-3">
            <div>
              <p className="text-sm font-semibold text-content-faint">
                {t('settings.billing.payAsYouGo.available')}
              </p>
              <p className="mt-2 text-2xl font-bold tracking-tight text-content-secondary">
                ${availableCredits.toFixed(2)}
              </p>
            </div>
            <div>
              <p className="text-sm font-semibold text-content-faint">
                {t('settings.billing.payAsYouGo.promotionalCredits')}
              </p>
              <p className="mt-2 text-xl font-bold tracking-tight text-content-secondary">
                ${promoCredits.toFixed(2)}
              </p>
            </div>
            <div>
              <p className="text-sm font-semibold text-content-faint">
                {t('settings.billing.payAsYouGo.topUpBalance')}
              </p>
              <p className="mt-2 text-xl font-bold tracking-tight text-content-secondary">
                ${teamTopupCredits.toFixed(2)}
              </p>
            </div>
          </div>
        ) : isLoadingCredits ? (
          <div className="mt-5 grid gap-3 sm:grid-cols-3">
            {[0, 1, 2].map(index => (
              <div key={index} className="h-24 rounded-2xl bg-surface-subtle animate-pulse" />
            ))}
          </div>
        ) : (
          <p className="mt-5 text-sm text-content-muted">
            {t('settings.billing.payAsYouGo.unableToLoad')}
          </p>
        )}
      </div>
      <div className="rounded-lg bg-surface p-6 shadow-[0_24px_70px_rgba(15,23,42,0.06)] ring-1 ring-neutral-950/5">
        <h3 className="font-headline text-xl font-bold tracking-tight text-content">
          {t('settings.billing.payAsYouGo.chooseTopUpTitle')}
        </h3>
        <p className="mt-1 text-sm text-content-muted">
          {t('settings.billing.payAsYouGo.chooseTopUpDesc')}
        </p>

        <div className="mt-6 grid gap-3 sm:grid-cols-3">
          {[5, 10, 25].map(amount => (
            <button
              key={amount}
              onClick={() => onTopUp(amount)}
              disabled={isToppingUp}
              className="group rounded-2xl border border-primary-200 dark:border-primary-500/30 bg-primary-50/50 px-4 py-5 text-center transition-all hover:border-primary-200 dark:border-primary-500/30 disabled:cursor-not-allowed disabled:opacity-50">
              <div className="text-2xl font-bold tracking-tight text-primary-600 dark:text-primary-300">
                {isToppingUp ? t('settings.billing.payAsYouGo.opening') : `$${amount.toFixed(2)}`}
              </div>
              <div className="mt-1 text-[11px] font-semibold text-content-faint">
                {t('settings.billing.payAsYouGo.topUpCredits')}
              </div>
            </button>
          ))}
        </div>

        <div className="mt-4 rounded-2xl border border-line bg-surface-muted p-4">
          <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto]">
            <div>
              <label
                htmlFor="billing-custom-top-up"
                className="text-[11px] font-semibold uppercase tracking-[0.24em] text-content-faint">
                {t('settings.billing.payAsYouGo.customAmount')}
              </label>
              <div className="mt-2 flex items-center">
                <span className="pl-3 text-sm font-semibold text-content-muted pointer-events-none">
                  $
                </span>
                <SettingsTextField
                  id="billing-custom-top-up"
                  type="number"
                  min="1"
                  step="0.01"
                  inputMode="decimal"
                  value={customTopUpAmount}
                  onChange={e => setCustomTopUpAmount(e.target.value)}
                  onKeyDown={e => {
                    if (e.key === 'Enter') handleCustomTopUp();
                  }}
                  placeholder={t('settings.billing.payAsYouGo.enterAmount')}
                  className="-ml-1 flex-1"
                />
              </div>
              <p className="mt-2 text-xs text-content-muted">
                {t('settings.billing.payAsYouGo.chooseTopUpDesc')}
              </p>
            </div>
            <Button
              type="button"
              variant="primary"
              size="md"
              onClick={handleCustomTopUp}
              disabled={!customTopUpAmountValid || isToppingUp}
              className="lg:self-end">
              {isToppingUp
                ? t('settings.billing.payAsYouGo.opening')
                : t('settings.billing.payAsYouGo.chargeCustomAmount')}
            </Button>
          </div>
        </div>
      </div>
    </>
  );
};

export default PayAsYouGoCard;
