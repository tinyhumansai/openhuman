import type { AutoRechargeSettings, CreditBalance, SavedCard } from '../../../../services/api/creditsApi';
import PayAsYouGoCard from './PayAsYouGoCard';
import AutoRechargeSection from './AutoRechargeSection';

interface BillingPaymentsTabProps {
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
  isLoadingCredits: boolean;
  isToppingUp: boolean;
  onAddCard: () => void;
  onArSave: () => void;
  onArToggle: () => void;
  onBalanceRefresh: () => void;
  onDeleteCard: (paymentMethodId: string) => void;
  onSetDefault: (paymentMethodId: string) => void;
  onTopUp: (amountUsd: number) => void;
  setArAmount: (value: number) => void;
  setArError: (value: string | null) => void;
  setArThreshold: (value: number) => void;
  setArWeeklyLimit: (value: number) => void;
  setConfirmDeleteId: (value: string | null) => void;
  settingDefaultId: string | null;
}

export default function BillingPaymentsTab({
  arAmount,
  arDirty,
  arError,
  arLoading,
  arSaving,
  arSettings,
  arThreshold,
  arWeeklyLimit,
  availableCredits,
  cards,
  cardsLoading,
  confirmDeleteId,
  creditBalance,
  deletingCardId,
  isLoadingCredits,
  isToppingUp,
  onAddCard,
  onArSave,
  onArToggle,
  onBalanceRefresh,
  onDeleteCard,
  onSetDefault,
  onTopUp,
  setArAmount,
  setArError,
  setArThreshold,
  setArWeeklyLimit,
  setConfirmDeleteId,
  settingDefaultId,
}: BillingPaymentsTabProps) {
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
    </>
  );
}
