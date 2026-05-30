import { QRCodeSVG } from 'qrcode.react';
import { useCallback, useEffect, useRef, useState } from 'react';

import { balanceNetworkLabel } from '../../../../features/wallet/walletDisplay';
import { useT } from '../../../../lib/i18n/I18nContext';
import type { BalanceInfo } from '../../../../services/walletApi';
import { ModalShell } from '../../../ui/ModalShell';

interface ReceiveModalProps {
  balance: BalanceInfo;
  onClose: () => void;
}

/**
 * Receive modal — renders the derived address for the selected chain/network as
 * a QR code plus a copyable string. Receiving is read-only: no signing, no RPC.
 * For EVM the same address works across every EVM network.
 */
const ReceiveModal = ({ balance, onClose }: ReceiveModalProps) => {
  const { t } = useT();
  const networkLabel = balanceNetworkLabel(balance);
  const [copied, setCopied] = useState(false);
  const copyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Clear the "Copied" reset timer if the modal unmounts before it fires.
  useEffect(
    () => () => {
      if (copyTimerRef.current !== null) clearTimeout(copyTimerRef.current);
    },
    []
  );

  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(balance.address);
      setCopied(true);
      if (copyTimerRef.current !== null) clearTimeout(copyTimerRef.current);
      copyTimerRef.current = setTimeout(() => setCopied(false), 2000);
    } catch {
      // Clipboard unavailable; ignore.
    }
  }, [balance.address]);

  return (
    <ModalShell
      onClose={onClose}
      titleId="wallet-receive-title"
      title={t('walletBalances.receive')}
      subtitle={networkLabel}>
      <div className="flex flex-col items-center gap-4">
        <p className="text-xs text-stone-500 dark:text-neutral-400 text-center leading-relaxed">
          {t('walletReceive.scanHint')}
        </p>
        <div className="rounded-xl bg-white p-3 border border-stone-200" data-testid="receive-qr">
          <QRCodeSVG
            value={balance.address}
            size={180}
            level="M"
            bgColor="#ffffff"
            fgColor="#1c1917"
          />
        </div>
        <div className="w-full">
          <span className="block text-[11px] font-medium text-stone-500 dark:text-neutral-400 mb-1">
            {t('walletReceive.addressLabel').replace('{network}', networkLabel)}
          </span>
          <div className="flex items-center gap-2 rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2">
            <span
              className="font-mono text-xs text-stone-700 dark:text-neutral-200 break-all"
              data-testid="receive-address">
              {balance.address}
            </span>
            <button
              type="button"
              onClick={() => void handleCopy()}
              className="shrink-0 text-xs font-medium text-primary-600 dark:text-primary-400 hover:text-primary-700 dark:hover:text-primary-300 transition-colors">
              {copied ? t('common.copied') : t('walletBalances.copyAddress')}
            </button>
          </div>
        </div>
        <div className="w-full rounded-xl bg-amber-50 dark:bg-amber-500/10 border border-amber-200 dark:border-amber-500/30 p-3">
          <p className="text-[11px] text-amber-700 dark:text-amber-300 leading-relaxed">
            {t('walletReceive.onlyChainWarning').replace('{network}', networkLabel)}
          </p>
        </div>
      </div>
    </ModalShell>
  );
};

export default ReceiveModal;
