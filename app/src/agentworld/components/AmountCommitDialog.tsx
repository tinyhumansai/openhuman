/**
 * AmountCommitDialog — amount-entry confirm dialog for x402 *commitments*
 * (bids and offers).
 *
 * Unlike X402ConfirmDialog (which gates an immediate on-chain spend), a bid/offer
 * is a signed authorization that only settles if accepted — so there is no
 * balance gate and no on-chain transfer at submit time. The user enters an
 * amount and submits; the parent owns the RPC call.
 */
import { useState } from 'react';

import Button from '../../components/ui/Button';
import { ModalShell } from '../../components/ui/ModalShell';

export interface AmountCommitDialogProps {
  /** Header title, e.g. "Bid on @handle". */
  title: string;
  /** Context line under the title. */
  subtitle?: string;
  /** Asset symbol shown next to the amount input (e.g. "USDC"). */
  asset: string;
  /** Submit-button label (e.g. "Place bid" / "Submit offer"). */
  submitLabel: string;
  /** Busy label while the commitment is in flight. */
  busyLabel?: string;
  busy?: boolean;
  /** Called with the entered amount (raw string) on submit. */
  onSubmit: (amount: string) => void;
  onCancel: () => void;
}

export default function AmountCommitDialog({
  title,
  subtitle,
  asset,
  submitLabel,
  busyLabel = 'Submitting…',
  busy = false,
  onSubmit,
  onCancel,
}: AmountCommitDialogProps) {
  const [amount, setAmount] = useState('');
  // Allow only digits (base-unit integer amount); empty disables submit.
  const sanitized = amount.replace(/[^0-9]/g, '');
  const canSubmit = sanitized.length > 0 && !busy;

  return (
    <ModalShell
      title={title}
      titleId="x402-commit-title"
      subtitle={subtitle}
      onClose={busy ? () => undefined : onCancel}
      maxWidthClassName="max-w-sm">
      <div className="space-y-4">
        <div>
          <label
            className="mb-1 block text-xs text-stone-400 dark:text-neutral-500"
            htmlFor="x402-commit-amount">
            Amount ({asset})
          </label>
          <input
            id="x402-commit-amount"
            data-testid="commit-amount-input"
            className="w-full rounded-md border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 outline-none focus:border-primary-500"
            inputMode="numeric"
            placeholder="0"
            value={amount}
            disabled={busy}
            onChange={e => setAmount(e.target.value.replace(/[^0-9]/g, ''))}
          />
        </div>

        <p className="text-xs text-stone-400 dark:text-neutral-500">
          This is a signed commitment — funds only move if it is accepted.
        </p>

        <div className="flex justify-end gap-2">
          <Button variant="secondary" size="sm" onClick={onCancel} disabled={busy}>
            Cancel
          </Button>
          <Button
            variant="primary"
            size="sm"
            data-testid="commit-submit"
            disabled={!canSubmit}
            onClick={() => onSubmit(sanitized)}>
            {busy ? busyLabel : submitLabel}
          </Button>
        </div>
      </div>
    </ModalShell>
  );
}
