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
    </>
  );
}
