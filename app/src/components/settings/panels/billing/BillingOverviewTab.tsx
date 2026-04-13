import type { AutoRechargeSettings, CreditBalance, CreditTransaction, SavedCard, TeamUsage } from '../../../../services/api/creditsApi';
import PayAsYouGoCard from './PayAsYouGoCard';
import SubscriptionPlans from './SubscriptionPlans';
import InferenceBudget from './InferenceBudget';
import AutoRechargeSection from './AutoRechargeSection';
import type { PlanTier } from '../../../../types/api';

interface BillingOverviewTabProps {
  arAmount: number;
  arDirty: boolean;
  arError: string | null;
  arLoading: boolean;
  arSaving: boolean;
  arSettings: AutoRechargeSettings | null;
  arThreshold: number;
  arWeeklyLimit: number;
  availableCredits: number;
  cards: SavedCard[];
  cardsLoading: boolean;
  confirmDeleteId: string | null;
  creditBalance: CreditBalance | null;
  deletingCardId: string | null;
  hasActive: boolean;
  isLoadingCredits: boolean;
  isPurchasing: boolean;
  isToppingUp: boolean;
  onAddCard: () => void;
  onArSave: () => void;
  onArToggle: () => void;
  onBalanceRefresh: () => void;
  onDeleteCard: (paymentMethodId: string) => void;
  onManageSubscription: () => void;
  onSetDefault: (paymentMethodId: string) => void;
  onTopUp: (amountUsd: number) => void;
  onUpgrade: (tier: PlanTier) => void;
  paymentConfirmed: boolean;
  paymentMethod: 'card' | 'crypto';
  purchasingTier: PlanTier | null;
  setArAmount: (value: number) => void;
  setArError: (value: string | null) => void;
  setArThreshold: (value: number) => void;
  setArWeeklyLimit: (value: number) => void;
  setBillingInterval: (value: 'monthly' | 'annual') => void;
  setConfirmDeleteId: (value: string | null) => void;
  setPaymentMethod: (value: 'card' | 'crypto') => void;
  settingDefaultId: string | null;
  teamUsage: TeamUsage | null;
  transactionRows: CreditTransaction[];
  billingInterval: 'monthly' | 'annual';
  currentTier: PlanTier;
}

export default function BillingOverviewTab({
  arAmount,
  arDirty,
  arError,
  arLoading,
  arSaving,
  arSettings,
  arThreshold,
  arWeeklyLimit,
  availableCredits,
  billingInterval,
  cards,
  cardsLoading,
  confirmDeleteId,
  creditBalance,
  currentTier,
  deletingCardId,
  hasActive,
  isLoadingCredits,
  isPurchasing,
  isToppingUp,
  onAddCard,
  onArSave,
  onArToggle,
  onBalanceRefresh,
  onDeleteCard,
  onManageSubscription,
  onSetDefault,
  onTopUp,
  onUpgrade,
  paymentConfirmed,
  paymentMethod,
  purchasingTier,
  setArAmount,
  setArError,
  setArThreshold,
  setArWeeklyLimit,
  setBillingInterval,
  setConfirmDeleteId,
  setPaymentMethod,
  settingDefaultId,
  teamUsage,
  transactionRows,
}: BillingOverviewTabProps) {
  return (
    <>
      <section className="space-y-4">
        <div className="flex items-end justify-between gap-3">
          <div>
            <h3 className="font-headline text-2xl font-bold tracking-tight text-stone-950">
              Credits & add-ons
            </h3>
            <p className="mt-1 text-sm text-stone-500">
              Keep your workspace funded with prepaid credits and coupon redemptions.
            </p>
          </div>
          <p className="text-sm font-semibold text-primary-600">
            Current balance: ${availableCredits.toFixed(2)}
          </p>
        </div>
        <PayAsYouGoCard
          creditBalance={creditBalance}
          isLoadingCredits={isLoadingCredits}
          isToppingUp={isToppingUp}
          onTopUp={onTopUp}
          onBalanceRefresh={onBalanceRefresh}
        />
      </section>

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

      <section className="space-y-4">
        <div>
          <h3 className="font-headline text-2xl font-bold tracking-tight text-stone-950">
            Usage budget
          </h3>
          <p className="mt-1 text-sm text-stone-500">
            Track included plan usage before credits begin to cover overage.
          </p>
        </div>
        <InferenceBudget teamUsage={teamUsage} isLoadingCredits={isLoadingCredits} />
      </section>

      <section className="space-y-4">
        <div className="flex items-center justify-between gap-3">
          <div>
            <h3 className="font-headline text-2xl font-bold tracking-tight text-stone-950">
              Payment methods
            </h3>
            <p className="mt-1 text-sm text-stone-500">
              Manage saved cards and auto-recharge thresholds.
            </p>
          </div>
        </div>
        <AutoRechargeSection
          arSettings={arSettings}
          arLoading={arLoading}
          arError={arError}
          arSaving={arSaving}
          arThreshold={arThreshold}
          arAmount={arAmount}
          arWeeklyLimit={arWeeklyLimit}
          arDirty={arDirty}
          setArThreshold={setArThreshold}
          setArAmount={setArAmount}
          setArWeeklyLimit={setArWeeklyLimit}
          setArError={setArError}
          onArToggle={onArToggle}
          onArSave={onArSave}
          cards={cards}
          cardsLoading={cardsLoading}
          confirmDeleteId={confirmDeleteId}
          deletingCardId={deletingCardId}
          settingDefaultId={settingDefaultId}
          setConfirmDeleteId={setConfirmDeleteId}
          onSetDefault={onSetDefault}
          onDeleteCard={onDeleteCard}
          onAddCard={onAddCard}
        />
      </section>

      <section className="space-y-4">
        <div className="flex items-center justify-between gap-3">
          <div>
            <h3 className="font-headline text-2xl font-bold tracking-tight text-stone-950">
              Recent invoices
            </h3>
            <p className="mt-1 text-sm text-stone-500">
              A quick view of recent billing activity from your credit ledger.
            </p>
          </div>
          {hasActive && (
            <button
              onClick={onManageSubscription}
              className="text-sm font-semibold text-primary-600 transition-colors hover:text-primary-700">
              Open billing portal
            </button>
          )}
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
    </>
  );
}
