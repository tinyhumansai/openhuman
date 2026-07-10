import { useT } from '../../../../lib/i18n/I18nContext';
import type { CreditTransaction } from '../../../../services/api/creditsApi';
import Button from '../../../ui/Button';

interface BillingHistoryTabProps {
  hasActive: boolean;
  onManageSubscription: () => void;
  transactionRows: CreditTransaction[];
}

export default function BillingHistoryTab({
  hasActive,
  onManageSubscription,
  transactionRows,
}: BillingHistoryTabProps) {
  const { t } = useT();
  return (
    <section className="space-y-4">
      <div className="flex flex-col gap-2 rounded-2xl bg-surface p-4 border border-line">
        <h3 className="font-headline text-2xl font-bold tracking-tight text-stone-950 dark:text-content">
          {t('settings.billing.history.title')}
        </h3>
        <p className="mt-1 text-sm text-content-muted">{t('settings.billing.history.desc')}</p>
        <div className="flex items-center justify-between gap-3">
          {hasActive && (
            <Button
              variant="tertiary"
              size="sm"
              onClick={onManageSubscription}
              className="px-0 font-semibold text-primary-600 hover:bg-transparent hover:text-primary-700 dark:text-primary-300">
              {t('settings.billing.history.openPortal')}
            </Button>
          )}
        </div>
      </div>
      <div className="overflow-hidden rounded-[28px] bg-surface shadow-[0_24px_70px_rgba(15,23,42,0.06)] ring-1 ring-stone-950/5">
        {transactionRows.length > 0 ? (
          <div className="divide-y divide-line-subtle dark:divide-neutral-800">
            {transactionRows.map(transaction => {
              const isEarn = transaction.type === 'EARN';
              return (
                <div
                  key={transaction.id}
                  className="grid gap-3 px-5 py-4 text-sm sm:grid-cols-[1.3fr_0.8fr_0.7fr_0.8fr] sm:items-center">
                  <div>
                    <p className="font-semibold text-stone-950 dark:text-content">
                      {transaction.action}
                    </p>
                    <p className="mt-1 text-xs text-content-muted">
                      {new Date(transaction.createdAt).toLocaleDateString(undefined, {
                        month: 'short',
                        day: 'numeric',
                        year: 'numeric',
                      })}
                    </p>
                  </div>
                  <div className="text-content-muted">{transaction.type}</div>
                  <div
                    className={`font-semibold ${isEarn ? 'text-sage-600 dark:text-sage-300' : 'text-stone-950 dark:text-content'}`}>
                    {isEarn ? '+' : '-'}${Math.abs(transaction.amountUsd).toFixed(2)}
                  </div>
                  <div className="sm:text-right">
                    <span className="rounded-full bg-surface-subtle px-3 py-1 text-xs font-semibold uppercase tracking-[0.18em] text-content-muted">
                      {t('settings.billing.history.posted')}
                    </span>
                  </div>
                </div>
              );
            })}
          </div>
        ) : (
          <div className="px-5 py-8 text-sm text-content-muted">
            {t('settings.billing.history.empty')}
          </div>
        )}
      </div>
    </section>
  );
}
