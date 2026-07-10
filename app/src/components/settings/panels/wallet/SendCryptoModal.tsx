import { useCallback, useMemo, useState } from 'react';

import {
  balanceNetworkLabel,
  fromSmallestUnit,
  toSmallestUnit,
} from '../../../../features/wallet/walletDisplay';
import { useT } from '../../../../lib/i18n/I18nContext';
import {
  type BalanceInfo,
  executePrepared,
  type ExecutionResult,
  type PreparedTransaction,
  prepareTransfer,
} from '../../../../services/walletApi';
import Button from '../../../ui/Button';
import { ModalShell } from '../../../ui/ModalShell';

interface SendCryptoModalProps {
  balance: BalanceInfo;
  onClose: () => void;
  /** Called after a successful broadcast so the panel can refresh balances. */
  onSuccess: () => void;
}

type Step = 'form' | 'review' | 'sending' | 'done';

/** Truncate a hash/address to `0x1234…abcd` for compact display. */
function truncate(value: string): string {
  if (value.length <= 14) return value;
  return `${value.slice(0, 8)}…${value.slice(-6)}`;
}

/**
 * Send modal — drives the wallet's prepare → confirm → execute flow for the
 * native asset of the selected balance row. `prepareTransfer` builds a quote
 * (with the simulated fee) that the user reviews before `executePrepared`
 * signs locally and broadcasts. Native asset only; token sends are a follow-up.
 */
const SendCryptoModal = ({ balance, onClose, onSuccess }: SendCryptoModalProps) => {
  const { t } = useT();
  const networkLabel = balanceNetworkLabel(balance);

  const [step, setStep] = useState<Step>('form');
  const [recipient, setRecipient] = useState('');
  const [amount, setAmount] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [prepared, setPrepared] = useState<PreparedTransaction | null>(null);
  const [result, setResult] = useState<ExecutionResult | null>(null);

  const feeFormatted = useMemo(() => {
    if (!prepared) return null;
    return fromSmallestUnit(prepared.estimatedFeeRaw, balance.decimals);
  }, [prepared, balance.decimals]);

  const handleReview = useCallback(async () => {
    setError(null);
    let amountRaw: string;
    try {
      amountRaw = toSmallestUnit(amount, balance.decimals);
    } catch {
      // toSmallestUnit throws dev-facing messages; surface a translated one.
      setError(t('walletSend.invalidAmount'));
      return;
    }
    if (amountRaw === '0') {
      setError(t('walletSend.invalidAmount'));
      return;
    }
    if (recipient.trim() === '') {
      setError(t('walletSend.recipientRequired'));
      return;
    }
    setBusy(true);
    try {
      const quote = await prepareTransfer({
        chain: balance.chain,
        toAddress: recipient.trim(),
        amountRaw,
        evmNetwork: balance.evmNetwork,
      });
      setPrepared(quote);
      setStep('review');
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.debug('[walletSend] prepare failed:', message);
      setError(message || t('walletSend.genericError'));
    } finally {
      setBusy(false);
    }
  }, [amount, recipient, balance, t]);

  const handleConfirm = useCallback(async () => {
    if (!prepared) return;
    setError(null);
    setBusy(true);
    setStep('sending');
    try {
      const executed = await executePrepared(prepared.quoteId);
      setResult(executed);
      setStep('done');
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.debug('[walletSend] execute failed:', message);
      setError(message || t('walletSend.genericError'));
      setStep('review');
    } finally {
      setBusy(false);
    }
  }, [prepared, t]);

  const handleDone = useCallback(() => {
    onSuccess();
    onClose();
  }, [onSuccess, onClose]);

  const fieldClass =
    'w-full rounded-lg border border-line-strong bg-surface px-3 py-2 text-sm text-content placeholder-content-faint focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500';

  return (
    <ModalShell
      onClose={onClose}
      titleId="wallet-send-title"
      title={t('walletBalances.send')}
      subtitle={`${networkLabel} · ${balance.assetSymbol}`}>
      {error && (
        <div
          role="alert"
          className="mb-3 rounded-lg bg-coral-50 dark:bg-coral-500/10 border border-coral-200 dark:border-coral-500/30 px-3 py-2 text-xs text-coral-700 dark:text-coral-300">
          {error}
        </div>
      )}

      {step === 'form' && (
        <div className="flex flex-col gap-3">
          <div className="flex items-center justify-between rounded-lg bg-surface-muted px-3 py-2 text-xs">
            <span className="text-content-muted">{t('walletSend.available')}</span>
            <span className="font-mono font-medium text-content">
              {balance.formatted} {balance.assetSymbol}
            </span>
          </div>
          <label className="flex flex-col gap-1">
            <span className="text-xs font-medium text-content-secondary">
              {t('walletSend.recipient')}
            </span>
            <input
              type="text"
              value={recipient}
              onChange={e => setRecipient(e.target.value)}
              placeholder={t('walletSend.recipientPlaceholder')}
              spellCheck={false}
              autoComplete="off"
              className={`${fieldClass} font-mono`}
              data-testid="send-recipient"
            />
          </label>
          <label className="flex flex-col gap-1">
            <span className="text-xs font-medium text-content-secondary">
              {t('walletSend.amount')}
            </span>
            <div className="relative">
              <input
                type="text"
                inputMode="decimal"
                value={amount}
                onChange={e => setAmount(e.target.value)}
                placeholder="0.0"
                className={`${fieldClass} pr-16 font-mono`}
                data-testid="send-amount"
              />
              <span className="absolute right-3 top-1/2 -translate-y-1/2 text-xs font-medium text-content-faint">
                {balance.assetSymbol}
              </span>
            </div>
          </label>
          <Button
            type="button"
            onClick={() => void handleReview()}
            disabled={busy}
            className="w-full"
            data-testid="send-review">
            {busy ? t('walletSend.preparing') : t('walletSend.review')}
          </Button>
        </div>
      )}

      {step === 'review' && prepared && (
        <div className="flex flex-col gap-3">
          <p className="text-xs text-content-muted leading-relaxed">
            {t('walletSend.confirmHint')}
          </p>
          <dl className="rounded-xl border border-line divide-y divide-line-subtle dark:divide-neutral-800 text-xs">
            <div className="flex items-center justify-between px-3 py-2">
              <dt className="text-content-muted">{t('walletSend.amount')}</dt>
              <dd className="font-mono font-medium text-content">
                {prepared.amountFormatted} {prepared.assetSymbol}
              </dd>
            </div>
            <div className="flex items-center justify-between px-3 py-2">
              <dt className="text-content-muted">{t('walletSend.recipient')}</dt>
              <dd className="font-mono text-content">{truncate(prepared.toAddress)}</dd>
            </div>
            <div className="flex items-center justify-between px-3 py-2">
              <dt className="text-content-muted">{t('walletSend.estimatedFee')}</dt>
              <dd className="font-mono text-content" data-testid="send-fee">
                {feeFormatted} {balance.assetSymbol}
              </dd>
            </div>
          </dl>
          {prepared.notes.length > 0 && (
            <ul className="list-disc pl-4 text-[11px] text-content-muted space-y-0.5">
              {prepared.notes.map((note, i) => (
                <li key={i}>{note}</li>
              ))}
            </ul>
          )}
          <div className="flex gap-2">
            <Button
              type="button"
              variant="secondary"
              onClick={() => {
                setStep('form');
                setPrepared(null);
              }}
              disabled={busy}
              className="flex-1">
              {t('common.back')}
            </Button>
            <Button
              type="button"
              onClick={() => void handleConfirm()}
              disabled={busy}
              className="flex-1"
              data-testid="send-confirm">
              {t('walletSend.confirmSend')}
            </Button>
          </div>
        </div>
      )}

      {step === 'sending' && (
        <div className="flex flex-col items-center gap-3 py-8 text-content-muted">
          <svg className="w-6 h-6 animate-spin" fill="none" viewBox="0 0 24 24">
            <circle
              className="opacity-25"
              cx="12"
              cy="12"
              r="10"
              stroke="currentColor"
              strokeWidth="4"
            />
            <path
              className="opacity-75"
              fill="currentColor"
              d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
            />
          </svg>
          <span className="text-sm">{t('walletSend.sending')}</span>
        </div>
      )}

      {step === 'done' && result && (
        <div className="flex flex-col items-center gap-3 py-2 text-center">
          <div className="w-12 h-12 rounded-full bg-sage-100 dark:bg-sage-500/15 flex items-center justify-center">
            <svg
              className="w-6 h-6 text-sage-600 dark:text-sage-400"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
            </svg>
          </div>
          <p className="text-sm font-medium text-content">{t('walletSend.sent')}</p>
          <div className="w-full rounded-xl border border-line bg-surface-muted px-3 py-2">
            <span className="block text-[11px] text-content-muted mb-0.5">
              {t('walletSend.txHash')}
            </span>
            <span
              className="font-mono text-xs text-content-secondary break-all"
              data-testid="send-tx-hash">
              {result.transactionHash}
            </span>
          </div>
          {result.explorerUrl && (
            <a
              href={result.explorerUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="text-xs font-medium text-primary-600 dark:text-primary-400 hover:text-primary-700 dark:hover:text-primary-300">
              {t('walletSend.viewExplorer')}
            </a>
          )}
          <Button type="button" onClick={handleDone} className="mt-1 w-full">
            {t('walletSend.done')}
          </Button>
        </div>
      )}
    </ModalShell>
  );
};

export default SendCryptoModal;
