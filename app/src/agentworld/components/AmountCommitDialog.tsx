/**
 * AmountCommitDialog — amount-entry dialog for x402 *commitments* (bids/offers).
 *
 * Unlike X402ConfirmDialog (which gates an immediate on-chain spend), a bid/offer
 * is a signed authorization that only settles if accepted — so there is no
 * balance gate and no on-chain transfer at submit time. The user enters an
 * amount; the parent owns the next step (a confirm review, then the RPC).
 *
 * UX: the input accepts a HUMAN decimal amount (e.g. "1.5" USDC). It is converted
 * to base units (× 10^decimals) before `onSubmit`, so the user never has to type
 * raw lamport-style integers and the downstream RPC contract (a base-units
 * string) is unchanged.
 */
import { useState } from 'react';

import Button from '../../components/ui/Button';
import { ModalShell } from '../../components/ui/ModalShell';
import { useT } from '../../lib/i18n/I18nContext';

export interface AmountCommitDialogProps {
  /** Header title, e.g. "Bid on @handle". */
  title: string;
  /** Context line under the title. */
  subtitle?: string;
  /** Asset symbol shown next to the amount input (e.g. "USDC"). */
  asset: string;
  /** Decimals for the asset (USDC = 6). Used to scale the human amount → base units. */
  decimals: number;
  /** Submit-button label (e.g. "Continue" → review step). */
  submitLabel: string;
  /** Busy label while the next step is in flight. */
  busyLabel?: string;
  busy?: boolean;
  /** Called with the amount in BASE units (string) on submit. */
  onSubmit: (amount: string) => void;
  onCancel: () => void;
}

export interface ParsedAmount {
  /** Amount in base units, ready for the RPC. Only set when `valid`. */
  base: string | null;
  valid: boolean;
  /** i18n key for the validation problem, or null when valid / empty. */
  errorKey: string | null;
}

/**
 * Parse a human decimal amount string into base units.
 *
 * Rules:
 * - empty → not valid, no error (submit just stays disabled).
 * - more fractional digits than `decimals` → invalid (`tooManyDecimals`).
 * - zero / non-positive → invalid (`mustBePositive`).
 * - otherwise → base-units integer string with no leading zeros.
 *
 * Input is assumed already sanitized to `[0-9.]` with at most one dot (the
 * onChange handler enforces that), but we re-validate defensively.
 */
export function parseHumanAmount(input: string, decimals: number): ParsedAmount {
  const trimmed = input.trim();
  if (trimmed === '' || trimmed === '.') {
    return { base: null, valid: false, errorKey: null };
  }
  // Reject anything that isn't digits + at most one dot.
  if (!/^\d*\.?\d*$/.test(trimmed)) {
    return { base: null, valid: false, errorKey: 'agentWorld.trading.amountInvalid' };
  }
  const [wholeRaw, fracRaw = ''] = trimmed.split('.');
  if (fracRaw.length > decimals) {
    return { base: null, valid: false, errorKey: 'agentWorld.trading.amountTooManyDecimals' };
  }
  const whole = wholeRaw === '' ? '0' : wholeRaw;
  const frac = fracRaw.padEnd(decimals, '0');
  // Concatenate then strip leading zeros (BigInt-safe; keeps it exact).
  const combined = `${whole}${frac}`.replace(/^0+(?=\d)/, '');
  let base: bigint;
  try {
    base = BigInt(combined);
  } catch {
    return { base: null, valid: false, errorKey: 'agentWorld.trading.amountInvalid' };
  }
  if (base <= 0n) {
    return { base: null, valid: false, errorKey: 'agentWorld.trading.amountMustBePositive' };
  }
  return { base: base.toString(), valid: true, errorKey: null };
}

/** Keep only digits + a single decimal point as the user types. */
function sanitizeDecimalInput(value: string): string {
  // Drop everything except digits and dots, then collapse to a single dot.
  const cleaned = value.replace(/[^0-9.]/g, '');
  const firstDot = cleaned.indexOf('.');
  if (firstDot === -1) return cleaned;
  return cleaned.slice(0, firstDot + 1) + cleaned.slice(firstDot + 1).replace(/\./g, '');
}

export default function AmountCommitDialog({
  title,
  subtitle,
  asset,
  decimals,
  submitLabel,
  busyLabel,
  busy = false,
  onSubmit,
  onCancel,
}: AmountCommitDialogProps) {
  const { t } = useT();
  const [amount, setAmount] = useState('');
  const parsed = parseHumanAmount(amount, decimals);
  const canSubmit = parsed.valid && !busy;

  function handleSubmit() {
    if (!parsed.valid || parsed.base == null) return;
    console.debug('[agentworld:commit-amount] submit', {
      asset,
      decimals,
      human: amount,
      base: parsed.base,
    });
    onSubmit(parsed.base);
  }

  return (
    <ModalShell
      title={title}
      titleId="x402-commit-title"
      subtitle={subtitle}
      onClose={busy ? () => undefined : onCancel}
      maxWidthClassName="max-w-sm">
      <div className="space-y-4">
        <div>
          <label className="mb-1 block text-xs text-content-faint" htmlFor="x402-commit-amount">
            {t('agentWorld.trading.amountLabel', 'Amount')} ({asset})
          </label>
          <input
            id="x402-commit-amount"
            data-testid="commit-amount-input"
            className="w-full rounded-md border border-line-strong bg-surface px-3 py-2 text-sm text-content outline-none focus:border-primary-500"
            inputMode="decimal"
            placeholder="0.0"
            value={amount}
            disabled={busy}
            onChange={e => setAmount(sanitizeDecimalInput(e.target.value))}
          />
          {parsed.errorKey && (
            <p className="mt-1 text-xs text-coral-500" data-testid="commit-amount-error">
              {parsed.errorKey === 'agentWorld.trading.amountTooManyDecimals'
                ? t(
                    'agentWorld.trading.amountTooManyDecimals',
                    `Use at most ${decimals} decimal places.`
                  )
                : parsed.errorKey === 'agentWorld.trading.amountMustBePositive'
                  ? t(
                      'agentWorld.trading.amountMustBePositive',
                      'Enter an amount greater than zero.'
                    )
                  : t('agentWorld.trading.amountInvalid', 'Enter a valid amount.')}
            </p>
          )}
        </div>

        <p className="text-xs text-content-faint">
          {t(
            'agentWorld.trading.commitSettleNote',
            'This is a signed commitment — funds only move if it is accepted.'
          )}
        </p>

        <div className="flex justify-end gap-2">
          <Button variant="secondary" size="sm" onClick={onCancel} disabled={busy}>
            {t('agentWorld.trading.cancel', 'Cancel')}
          </Button>
          <Button
            variant="primary"
            size="sm"
            data-testid="commit-submit"
            disabled={!canSubmit}
            onClick={handleSubmit}>
            {busy ? (busyLabel ?? t('agentWorld.trading.submitting', 'Submitting…')) : submitLabel}
          </Button>
        </div>
      </div>
    </ModalShell>
  );
}
