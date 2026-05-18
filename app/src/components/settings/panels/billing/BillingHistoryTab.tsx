import { useT } from '../../../../lib/i18n/I18nContext';
import type { CreditTransaction } from '../../../../services/api/creditsApi';

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
      <div className="flex flex-col gap-2 rounded-2xl bg-white dark:bg-neutral-900 p-4 border border-stone-200 dark:border-neutral-800">
        <h3 className="font-headline text-2xl font-bold tracking-tight text-stone-950 dark:text-neutral-50">
          {t('settings.billing.history.title')}
        </h3>
        <p className="mt-1 text-sm text-stone-500 dark:text-neutral-400">
          {t('settings.billing.history.desc')}
        </p>
        <div className="flex items-center justify-between gap-3">
          {hasActive && (
            <button
              onClick={onManageSubscription}
              className="text-sm font-semibold text-primary-600 dark:text-primary-300 transition-colors hover:text-primary-700 dark:text-primary-300">
              {t('settings.billing.history.openPortal')}
            </button>
          )}
        </div>
      </div>
      <div className="overflow-hidden rounded-[28px] bg-white dark:bg-neutral-900 shadow-[0_24px_70px_rgba(15,23,42,0.06)] ring-1 ring-stone-950/5">
        {transactionRows.length > 0 ? (
          <div className="divide-y divide-stone-100 dark:divide-neutral-800">
            {transactionRows.map(transaction => {
              const isEarn = transaction.type === 'EARN';
              return (
                <div
                  key={transaction.id}
                  className="grid gap-3 px-5 py-4 text-sm sm:grid-cols-[1.3fr_0.8fr_0.7fr_0.8fr] sm:items-center">
                  <div>
                    <p className="font-semibold text-stone-950 dark:text-neutral-50">
                      {transaction.action}
                    </p>
                    <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
                      {new Date(transaction.createdAt).toLocaleDateString(undefined, {
                        month: 'short',
                        day: 'numeric',
                        year: 'numeric',
                      })}
                    </p>
                  </div>
                  <div className="text-stone-500 dark:text-neutral-400">{transaction.type}</div>
                  <div
                    className={`font-semibold ${isEarn ? 'text-sage-600 dark:text-sage-300' : 'text-stone-950 dark:text-neutral-50'}`}>
                    {isEarn ? '+' : '-'}${Math.abs(transaction.amountUsd).toFixed(2)}
                  </div>
                  <div className="sm:text-right">
                    <span className="rounded-full bg-stone-100 dark:bg-neutral-800 px-3 py-1 text-xs font-semibold uppercase tracking-[0.18em] text-stone-500 dark:text-neutral-400">
                      {t('settings.billing.history.posted')}
                    </span>
                  </div>
                </div>
              );
            })}
          </div>
        ) : (
          <div className="px-5 py-8 text-sm text-stone-500 dark:text-neutral-400">
            {t('settings.billing.history.empty')}
          </div>
        )}
      </div>
    </section>
  );
}
