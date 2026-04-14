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
  return (
    <section className="space-y-4">
      <div className="flex flex-col gap-2 rounded-2xl bg-white p-4 border border-stone-200">
        <h3 className="font-headline text-2xl font-bold tracking-tight text-stone-950">
          Recent invoices
        </h3>
        <p className="mt-1 text-sm text-stone-500">
          A quick view of recent billing activity from your credit ledger.
        </p>
        <div className="flex items-center justify-between gap-3">
          {hasActive && (
            <button
              onClick={onManageSubscription}
              className="text-sm font-semibold text-primary-600 transition-colors hover:text-primary-700">
              Open billing portal
            </button>
          )}
        </div>
      </div>
      <div className="overflow-hidden rounded-[28px] bg-white shadow-[0_24px_70px_rgba(15,23,42,0.06)] ring-1 ring-stone-950/5">
        {transactionRows.length > 0 ? (
          <div className="divide-y divide-stone-100">
            {transactionRows.map(transaction => {
              const isEarn = transaction.type === 'EARN';
              return (
                <div
                  key={transaction.id}
                  className="grid gap-3 px-5 py-4 text-sm sm:grid-cols-[1.3fr_0.8fr_0.7fr_0.8fr] sm:items-center">
                  <div>
                    <p className="font-semibold text-stone-950">{transaction.action}</p>
                    <p className="mt-1 text-xs text-stone-500">
                      {new Date(transaction.createdAt).toLocaleDateString('en-US', {
                        month: 'short',
                        day: 'numeric',
                        year: 'numeric',
                      })}
                    </p>
                  </div>
                  <div className="text-stone-500">{transaction.type}</div>
                  <div className={`font-semibold ${isEarn ? 'text-sage-600' : 'text-stone-950'}`}>
                    {isEarn ? '+' : '-'}${Math.abs(transaction.amountUsd).toFixed(2)}
                  </div>
                  <div className="sm:text-right">
                    <span className="rounded-full bg-stone-100 px-3 py-1 text-xs font-semibold uppercase tracking-[0.18em] text-stone-500">
                      Posted
                    </span>
                  </div>
                </div>
              );
            })}
          </div>
        ) : (
          <div className="px-5 py-8 text-sm text-stone-500">
            No recent billing activity is available yet.
          </div>
        )}
      </div>
    </section>
  );
}
