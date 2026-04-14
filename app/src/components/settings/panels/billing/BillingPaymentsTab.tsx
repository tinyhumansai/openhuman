import type {
  AutoRechargeSettings,
  CreditBalance,
  SavedCard,
} from '../../../../services/api/creditsApi';
import AutoRechargeSection from './AutoRechargeSection';
import PayAsYouGoCard from './PayAsYouGoCard';

interface BillingPaymentsTabProps {
  arAmount: number;
  arDirty: boolean;
  arError: string | null;
  arLoading: boolean;
  arSaving: boolean;
  arSettings: AutoRechargeSettings | null;
  arThreshold: number;
  arWeeklyLimit: number;
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
  setArThreshold,
  setArWeeklyLimit,
  setConfirmDeleteId,
  settingDefaultId,
}: BillingPaymentsTabProps) {
  return (
    <>
      <div className="flex flex-col gap-2 rounded-2xl bg-white p-4 border border-stone-200">
        <h3 className="font-headline text-2xl font-bold tracking-tight text-stone-950">
          Top ups & Credits
        </h3>
        <p className="mt-1 text-sm text-stone-500">
          You can top up your credits if you ever exhaust your monthly budget or hit rate limits.
          Credits are consumed after any included subscription budget is exhausted.
        </p>
      </div>

      <section className="space-y-4">
        <PayAsYouGoCard
          creditBalance={creditBalance}
          isLoadingCredits={isLoadingCredits}
          isToppingUp={isToppingUp}
          onTopUp={onTopUp}
          onBalanceRefresh={onBalanceRefresh}
        />
      </section>

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
    </>
  );
}
