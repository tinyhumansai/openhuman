import { useState } from 'react';

import { type CreditBalance } from '../../../../services/api/creditsApi';

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
  // Backend `GET /payments/credits/balance` returns
  //   { promotionBalanceUsd, teamTopupUsd }
  // `promotionBalanceUsd` lives on the user document
  // (`IUserUsage.promotionBalanceUsd`) and unifies signup bonus, coupons,
  // and referral rewards. `teamTopupUsd` is the team-level paid top-up pool.
  // Together they make the pay-as-you-go spendable balance.
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
      <div className="rounded-lg bg-white p-6 shadow-[0_24px_70px_rgba(15,23,42,0.06)] ring-1 ring-stone-950/5">
        <h3 className="font-headline text-xl font-bold tracking-tight text-stone-950">
          Your Credit Balance
        </h3>
        <p className="mt-1 text-sm text-stone-500">
          You can top up your credits if you ever exhaust your monthly budget or hit rate limits.
          Credits are consumed after any included subscription budget is exhausted.
        </p>
        {creditBalance ? (
          <div className="grid mt-4 gap-3 sm:grid-cols-3">
            <div>
              <p className="text-sm font-semibold text-stone-400">Available</p>
              <p className="mt-2 text-2xl font-bold tracking-tight text-stone-600">
                ${availableCredits.toFixed(2)}
              </p>
            </div>
            <div>
              <p className="text-sm font-semibold text-stone-400">Promotional Credits</p>
              <p className="mt-2 text-xl font-bold tracking-tight text-stone-600">
                ${promoCredits.toFixed(2)}
              </p>
            </div>
            <div>
              <p className="text-sm font-semibold text-stone-400">Top-up Balance</p>
              <p className="mt-2 text-xl font-bold tracking-tight text-stone-600">
                ${teamTopupCredits.toFixed(2)}
              </p>
            </div>
          </div>
        ) : isLoadingCredits ? (
          <div className="mt-5 grid gap-3 sm:grid-cols-3">
            {[0, 1, 2].map(index => (
              <div key={index} className="h-24 rounded-2xl bg-stone-100 animate-pulse" />
            ))}
          </div>
        ) : (
          <p className="mt-5 text-sm text-stone-500">Unable to load balance.</p>
        )}
      </div>
      <div className="rounded-lg bg-white p-6 shadow-[0_24px_70px_rgba(15,23,42,0.06)] ring-1 ring-stone-950/5">
        <h3 className="font-headline text-xl font-bold tracking-tight text-stone-950">
          Choose a Top-up Amount
        </h3>
        <p className="mt-1 text-sm text-stone-500">
          Choose one of the preset amounts above or enter your own charge amount.
        </p>

        <div className="mt-6 grid gap-3 sm:grid-cols-3">
          {[5, 10, 25].map(amount => (
            <button
              key={amount}
              onClick={() => onTopUp(amount)}
              disabled={isToppingUp}
              className="group rounded-2xl border border-primary-200/50 bg-primary-50/50 px-4 py-5 text-center transition-all hover:border-primary-200 disabled:cursor-not-allowed disabled:opacity-50">
              <div className="text-2xl font-bold tracking-tight text-primary-600">
                {isToppingUp ? 'Opening…' : `$${amount.toFixed(2)}`}
              </div>
              <div className="mt-1 text-[11px] font-semibold text-stone-400">Top Up Credits</div>
            </button>
          ))}
        </div>

        <div className="mt-4 rounded-2xl border border-stone-200 bg-stone-50 p-4">
          <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_auto]">
            <div>
              <label
                htmlFor="billing-custom-top-up"
                className="text-[11px] font-semibold uppercase tracking-[0.24em] text-stone-400">
                Custom amount
              </label>
              <div className="mt-2 flex items-center rounded-2xl bg-white px-4 ring-1 ring-stone-200 focus-within:ring-2 focus-within:ring-primary-500/20">
                <span className="text-sm font-semibold text-stone-500">$</span>
                <input
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
                  placeholder="Enter amount"
                  className="w-full border-0 bg-transparent px-3 py-3 text-sm text-stone-900 placeholder:text-stone-400 focus:outline-none focus:ring-0"
                />
              </div>
              <p className="mt-2 text-xs text-stone-500">
                Choose one of the preset amounts above or enter your own charge amount.
              </p>
            </div>
            <button
              onClick={handleCustomTopUp}
              disabled={!customTopUpAmountValid || isToppingUp}
              className="rounded-2xl bg-stone-950 px-5 py-3 text-sm font-semibold text-white transition-colors hover:bg-primary-600 disabled:cursor-not-allowed disabled:opacity-50 lg:self-end">
              {isToppingUp ? 'Opening…' : 'Charge custom amount'}
            </button>
          </div>
        </div>
      </div>
    </>
  );
};

export default PayAsYouGoCard;
