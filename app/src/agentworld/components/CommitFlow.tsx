/**
 * CommitFlow — two-phase confirm-before-spend flow for x402 *commitments*
 * (identity bids and offers), giving them parity with the Buy flow.
 *
 * Flow:
 *   1. `amount`  — AmountCommitDialog: enter a human decimal amount (→ base units).
 *   2. `review`  — X402ConfirmDialog in `mode="commit"`: show amount + asset +
 *                  balance with a SOFT insufficient-funds warning (commitments
 *                  settle only on acceptance, so a low balance never blocks).
 *   3. `submit`  — call `apiClient.marketplace.bid|offer` and report the outcome.
 *
 * Unlike Buy, the bid/offer RPCs have no `confirmed:false` probe that returns a
 * wallet balance, and the multichain wallet RPC only reports NATIVE balances
 * (SOL), not the USDC token balance the commitment is denominated in. So the
 * review step renders with an UNKNOWN balance — X402ConfirmDialog shows a
 * "couldn't verify balance" note and still allows Confirm (honest: we don't
 * pretend the wallet can cover it; the backend is the authoritative gate).
 *
 * The component never touches Solana/x402 directly — every spend path goes
 * through the injected `submit` callback (→ apiClient.marketplace.*).
 */
import { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import AmountCommitDialog from './AmountCommitDialog';
import X402ConfirmDialog, { type X402WalletBalance } from './X402ConfirmDialog';

export type CommitKind = 'bid' | 'offer';

export interface CommitFlowProps {
  kind: CommitKind;
  /** Display name of the listing/handle (e.g. "@auction"). */
  name: string;
  /** Asset symbol shown in both dialogs (e.g. "USDC"). */
  asset: string;
  /** Decimals for `asset` (USDC = 6) — scales the human amount → base units. */
  decimals: number;
  /** Network label for the review step (transparency only). */
  network?: string;
  /**
   * Optional wallet balance for the review step. Bid/offer have no balance probe
   * today, so callers usually pass `null` → the dialog shows "couldn't verify".
   */
  balance?: X402WalletBalance | null;
  /** Wallet address shown (truncated) in the review step. */
  walletAddress?: string;
  /**
   * Perform the commitment. Receives the amount in BASE units. Resolves on
   * success, rejects on failure (CommitFlow surfaces both via callbacks).
   */
  submit: (amountBase: string) => Promise<void>;
  /** Fired after a successful commitment (parent shows the success banner). */
  onSuccess: () => void;
  /** Fired on failure with the error message (parent shows the error banner). */
  onError: (message: string) => void;
  /** Cancel/close the whole flow. */
  onClose: () => void;
}

type Phase =
  | { step: 'amount' }
  | { step: 'review'; amountBase: string }
  | { step: 'submitting'; amountBase: string };

export default function CommitFlow({
  kind,
  name,
  asset,
  decimals,
  network,
  balance = null,
  walletAddress = '',
  submit,
  onSuccess,
  onError,
  onClose,
}: CommitFlowProps) {
  const { t } = useT();
  const [phase, setPhase] = useState<Phase>({ step: 'amount' });

  // The i18n `t(key, fallback)` has NO interpolation, so the listing `name`
  // cannot be a translation param — we translate a PREFIX and compose with the
  // name in code. (Keys must exist in en.ts + every locale; a key with a `${name}`
  // fallback would always fall through to English and slip past i18n parity.)
  const amountTitle =
    kind === 'bid'
      ? `${t('agentWorld.trading.bidTitlePrefix')} ${name}`
      : `${t('agentWorld.trading.offerTitlePrefix')} ${name}`;
  const reviewTitle =
    kind === 'bid'
      ? `${t('agentWorld.trading.bidReviewTitlePrefix')} ${name}`
      : `${t('agentWorld.trading.offerReviewTitlePrefix')} ${name}`;
  const confirmLabel =
    kind === 'bid'
      ? t('agentWorld.trading.placeBid', 'Place bid')
      : t('agentWorld.trading.submitOffer', 'Submit offer');

  function handleAmount(amountBase: string) {
    console.debug('[agentworld:commit-flow] amount → review', { kind, name, amountBase });
    setPhase({ step: 'review', amountBase });
  }

  function handleConfirm() {
    if (phase.step !== 'review') return;
    const { amountBase } = phase;
    console.debug('[agentworld:commit-flow] confirm → submit', { kind, name, amountBase });
    setPhase({ step: 'submitting', amountBase });
    void submit(amountBase)
      .then(() => {
        console.debug('[agentworld:commit-flow] commit ok', { kind, name });
        onSuccess();
      })
      .catch((err: unknown) => {
        const message = String(err);
        console.debug('[agentworld:commit-flow] commit failed', { kind, name, message });
        onError(message);
      });
  }

  if (phase.step === 'amount') {
    return (
      <AmountCommitDialog
        title={amountTitle}
        subtitle={t(
          'agentWorld.trading.commitSettleNote',
          'A signed commitment — funds move only if it is accepted.'
        )}
        asset={asset}
        decimals={decimals}
        submitLabel={t('agentWorld.trading.continue', 'Continue')}
        onSubmit={handleAmount}
        onCancel={onClose}
      />
    );
  }

  return (
    <X402ConfirmDialog
      title={reviewTitle}
      subtitle={t(
        'agentWorld.trading.commitReviewSubtitle',
        'Review your commitment before submitting.'
      )}
      mode="commit"
      amount={phase.amountBase}
      asset={asset}
      network={network}
      balance={balance}
      walletAddress={walletAddress}
      busy={phase.step === 'submitting'}
      busyLabel={t('agentWorld.trading.submitting', 'Submitting…')}
      confirmLabel={confirmLabel}
      onConfirm={handleConfirm}
      onCancel={onClose}
    />
  );
}
