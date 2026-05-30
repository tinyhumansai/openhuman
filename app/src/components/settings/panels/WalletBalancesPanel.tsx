import { useCallback, useEffect, useRef, useState } from 'react';

import {
  balanceBadge,
  balanceKey,
  balanceNetworkLabel,
} from '../../../features/wallet/walletDisplay';
import { useT } from '../../../lib/i18n/I18nContext';
import {
  type BalanceInfo,
  type EvmNetwork,
  fetchWalletBalances,
  fetchWalletStatus,
  type WalletChain,
} from '../../../services/walletApi';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import ReceiveModal from './wallet/ReceiveModal';
import SendCryptoModal from './wallet/SendCryptoModal';

// ---------------------------------------------------------------------------
// Chain badge colours — each chain gets a distinct palette token combination
// that maps to the project's sage / amber / coral / ocean (primary) design
// language.  Tailwind class strings are kept literal so the build can detect
// them via static analysis.
// ---------------------------------------------------------------------------

const CHAIN_BADGE_CLASS: Record<string, string> = {
  evm: 'bg-primary-100 text-primary-700 dark:bg-primary-900/30 dark:text-primary-300',
  btc: 'bg-amber-100 text-amber-700 dark:bg-amber-900/30 dark:text-amber-300',
  solana: 'bg-sage-100 text-sage-700 dark:bg-sage-900/30 dark:text-sage-300',
  tron: 'bg-coral-100 text-coral-700 dark:bg-coral-900/30 dark:text-coral-300',
};

const badgeClassFor = (chain: WalletChain): string =>
  CHAIN_BADGE_CLASS[chain] ??
  'bg-stone-100 text-stone-700 dark:bg-neutral-800 dark:text-neutral-300';

// The rows rendered as placeholders before the wallet is set up, mirroring the
// configured layout (one EVM row per displayed network + BTC/Solana/Tron) so
// the preview matches what appears once a recovery phrase exists.
const PLACEHOLDER_ROWS: Array<{ chain: WalletChain; evmNetwork?: EvmNetwork; symbol: string }> = [
  { chain: 'evm', evmNetwork: 'ethereum_mainnet', symbol: 'ETH' },
  { chain: 'evm', evmNetwork: 'base_mainnet', symbol: 'ETH' },
  { chain: 'evm', evmNetwork: 'bsc_mainnet', symbol: 'BNB' },
  { chain: 'btc', symbol: 'BTC' },
  { chain: 'solana', symbol: 'SOL' },
  { chain: 'tron', symbol: 'TRX' },
];

/** Shorten an address to first 6 + last 4 characters: `0x1234…abcd`. */
function truncateAddress(address: string): string {
  if (address.length <= 12) return address;
  return `${address.slice(0, 6)}…${address.slice(-4)}`;
}

// ---------------------------------------------------------------------------
// BalanceRow — a single chain/network entry with Send / Receive actions
// ---------------------------------------------------------------------------

interface BalanceRowProps {
  balance: BalanceInfo;
  onSend: (balance: BalanceInfo) => void;
  onReceive: (balance: BalanceInfo) => void;
}

const BalanceRow = ({ balance, onSend, onReceive }: BalanceRowProps) => {
  const { t } = useT();
  const [copied, setCopied] = useState(false);
  // Tracks the most recent "Copied" timer so rapid re-clicks reset the 2s
  // window rather than stacking independent setTimeouts (the older one would
  // otherwise flip `copied` back to false while the newest click still wants
  // to show the checkmark).
  const copyResetTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (copyResetTimerRef.current !== null) {
        clearTimeout(copyResetTimerRef.current);
        copyResetTimerRef.current = null;
      }
    },
    []
  );

  const handleCopyAddress = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(balance.address);
      setCopied(true);
      if (copyResetTimerRef.current !== null) {
        clearTimeout(copyResetTimerRef.current);
      }
      copyResetTimerRef.current = setTimeout(() => {
        setCopied(false);
        copyResetTimerRef.current = null;
      }, 2000);
    } catch {
      // Clipboard unavailable (no permissions); silently skip.
    }
  }, [balance.address]);

  const badgeClass = badgeClassFor(balance.chain);
  const networkLabel = balanceNetworkLabel(balance);

  return (
    <div className="px-4 py-3">
      <div className="flex items-center gap-3">
        {/* Network badge */}
        <span
          className={`inline-flex items-center px-2 py-0.5 rounded-md text-xs font-semibold font-mono min-w-[3rem] justify-center shrink-0 ${badgeClass}`}>
          {balanceBadge(balance)}
        </span>

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="text-xs font-medium text-stone-700 dark:text-neutral-200 truncate">
              {networkLabel}
            </span>
            {balance.providerStatus !== 'ready' && (
              <span className="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium bg-amber-100 text-amber-700 dark:bg-amber-900/30 dark:text-amber-400">
                {t('walletBalances.providerMissing')}
              </span>
            )}
          </div>
          {/* Address + copy button */}
          <div className="flex items-center gap-1.5 min-w-0">
            <span className="font-mono text-[11px] text-stone-500 dark:text-neutral-400 truncate">
              {truncateAddress(balance.address)}
            </span>
            <button
              type="button"
              onClick={() => void handleCopyAddress()}
              aria-label={t('walletBalances.copyAddress')}
              className="shrink-0 text-stone-400 hover:text-stone-600 dark:text-neutral-500 dark:hover:text-neutral-300 transition-colors">
              {copied ? (
                <svg
                  className="w-3.5 h-3.5 text-sage-500"
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  strokeWidth={2.5}>
                  <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
                </svg>
              ) : (
                <svg
                  className="w-3.5 h-3.5"
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  strokeWidth={2}>
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"
                  />
                </svg>
              )}
            </button>
          </div>
        </div>

        {/* Amount */}
        <div className="text-right shrink-0">
          <span
            title={t('walletBalances.rawBalance').replace('{raw}', balance.raw)}
            className="text-sm font-medium text-stone-800 dark:text-neutral-100 font-mono">
            {balance.formatted}
          </span>
          <span className="ml-1 text-xs text-stone-500 dark:text-neutral-400">
            {balance.assetSymbol}
          </span>
        </div>
      </div>

      {/* Send / Receive actions */}
      <div className="flex gap-2 mt-2.5">
        <button
          type="button"
          onClick={() => onSend(balance)}
          className="flex-1 py-1.5 text-xs font-medium rounded-lg border border-stone-200 dark:border-neutral-700 text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 transition-colors"
          data-testid={`wallet-send-${balanceKey(balance)}`}>
          {t('walletBalances.send')}
        </button>
        <button
          type="button"
          onClick={() => onReceive(balance)}
          className="flex-1 py-1.5 text-xs font-medium rounded-lg border border-stone-200 dark:border-neutral-700 text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 transition-colors"
          data-testid={`wallet-receive-${balanceKey(balance)}`}>
          {t('walletBalances.receive')}
        </button>
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// ChainPlaceholderRow — shown per chain before the wallet is configured. There
// is no derived address or balance yet, so we render a muted "not set up" row
// to convey the wallet layout without fabricating data.
// ---------------------------------------------------------------------------

const ChainPlaceholderRow = ({
  chain,
  evmNetwork,
  symbol,
}: {
  chain: WalletChain;
  evmNetwork?: EvmNetwork;
  symbol: string;
}) => {
  const { t } = useT();
  const badgeClass = badgeClassFor(chain);

  return (
    <div className="flex items-center gap-3 px-4 py-3 opacity-70">
      <span
        className={`inline-flex items-center px-2 py-0.5 rounded-md text-xs font-semibold font-mono min-w-[3rem] justify-center shrink-0 ${badgeClass}`}>
        {balanceBadge({ chain, evmNetwork })}
      </span>
      <div className="min-w-0">
        <span className="block text-xs font-medium text-stone-400 dark:text-neutral-500 truncate">
          {balanceNetworkLabel({ chain, evmNetwork })}
        </span>
        <span className="font-mono text-[11px] text-stone-400 dark:text-neutral-500 truncate">
          {t('walletBalances.notSetUp')}
        </span>
      </div>
      <div className="flex-1" />
      <div className="text-right shrink-0">
        {/* Em dash placeholder — punctuation, not translatable copy. */}
        <span className="text-sm font-medium text-stone-400 dark:text-neutral-500 font-mono">
          —
        </span>
        <span className="ml-1 text-xs text-stone-400 dark:text-neutral-500">{symbol}</span>
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// WalletBalancesPanel — main panel
// ---------------------------------------------------------------------------

const WalletBalancesPanel = () => {
  const { t } = useT();
  const { navigateBack, navigateToSettings, breadcrumbs } = useSettingsNavigation();

  const [balances, setBalances] = useState<BalanceInfo[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // null = unknown (not yet loaded); false = wallet has no recovery phrase set
  // up yet, in which case we show a hint + placeholder rows instead of erroring.
  const [walletConfigured, setWalletConfigured] = useState<boolean | null>(null);
  // The balance row a Send / Receive modal is currently open for (null = none).
  const [sendTarget, setSendTarget] = useState<BalanceInfo | null>(null);
  const [receiveTarget, setReceiveTarget] = useState<BalanceInfo | null>(null);

  // Request-sequencing guard: a slower earlier request must not overwrite a
  // newer one. `loadBalances` can fire concurrently (mount + Refresh + Retry),
  // so we tag each call with a monotonic id and drop any response whose id no
  // longer matches the latest dispatched call.
  const latestRequestIdRef = useRef(0);

  const loadBalances = useCallback(async () => {
    const requestId = ++latestRequestIdRef.current;
    setLoading(true);
    setError(null);
    try {
      // Check setup state first: the core errors `wallet_balances` when no
      // recovery phrase is configured. Rather than blocking the panel on that,
      // detect it via the structured `configured` flag and fall through to the
      // hint + placeholder rows.
      const status = await fetchWalletStatus();
      if (requestId !== latestRequestIdRef.current) return;
      if (!status.configured) {
        setWalletConfigured(false);
        setBalances([]);
        return;
      }
      setWalletConfigured(true);
      const rows = await fetchWalletBalances();
      if (requestId !== latestRequestIdRef.current) return;
      setBalances(rows);
    } catch (err) {
      if (requestId !== latestRequestIdRef.current) return;
      const message = err instanceof Error ? err.message : String(err);
      // Log the raw backend phrasing for diagnostics; the UI surfaces a
      // translated, user-facing copy via `walletBalances.errorGeneric`.
      console.debug('[walletBalances] fetch failed:', message);
      setError(message);
    } finally {
      if (requestId === latestRequestIdRef.current) {
        setLoading(false);
      }
    }
  }, []);

  useEffect(() => {
    void loadBalances();
  }, [loadBalances]);

  const renderContent = () => {
    if (loading) {
      return (
        <div className="flex items-center justify-center gap-2 py-10 text-stone-500 dark:text-neutral-400">
          <svg className="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
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
          <span className="text-sm">{t('walletBalances.loading')}</span>
        </div>
      );
    }

    if (error) {
      return (
        <div className="px-4 py-4">
          <div
            role="alert"
            className="flex items-start gap-2.5 p-3 mb-4 rounded-xl bg-coral-50 dark:bg-coral-500/10 border border-coral-200 dark:border-coral-500/30">
            <svg
              className="w-4 h-4 text-coral-500 flex-shrink-0 mt-0.5"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={2}>
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"
              />
            </svg>
            <p className="text-xs text-coral-700 dark:text-coral-300 leading-relaxed">
              {t('walletBalances.errorGeneric')}
            </p>
          </div>
          <button
            type="button"
            onClick={() => void loadBalances()}
            className="btn-primary w-full py-2.5 text-sm font-medium rounded-xl">
            {t('walletBalances.retry')}
          </button>
        </div>
      );
    }

    // Wallet not set up yet: show a non-blocking hint plus placeholder rows so
    // the wallet layout is visible even before a recovery phrase exists.
    if (walletConfigured === false) {
      return (
        <div>
          <div className="px-4 pt-4 pb-3">
            <div
              role="status"
              className="flex items-start gap-2.5 p-3 rounded-xl bg-amber-50 dark:bg-amber-500/10 border border-amber-200 dark:border-amber-500/30">
              <svg
                className="w-4 h-4 text-amber-500 flex-shrink-0 mt-0.5"
                fill="none"
                viewBox="0 0 24 24"
                stroke="currentColor"
                strokeWidth={2}>
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"
                />
              </svg>
              <div className="flex-1 min-w-0">
                <p className="text-xs text-amber-700 dark:text-amber-300 leading-relaxed">
                  {t('walletBalances.setupHint')}
                </p>
                <button
                  type="button"
                  onClick={() => navigateToSettings('recovery-phrase')}
                  className="mt-2 text-xs font-medium text-primary-600 dark:text-primary-400 hover:text-primary-700 dark:hover:text-primary-300 transition-colors">
                  {t('walletBalances.setupCta')}
                </button>
              </div>
            </div>
          </div>
          <div className="divide-y divide-stone-100 dark:divide-neutral-800">
            {PLACEHOLDER_ROWS.map(row => (
              <ChainPlaceholderRow
                key={`${row.chain}-${row.evmNetwork ?? 'native'}`}
                chain={row.chain}
                evmNetwork={row.evmNetwork}
                symbol={row.symbol}
              />
            ))}
          </div>
        </div>
      );
    }

    if (balances !== null && balances.length === 0) {
      return (
        <div className="px-4 py-8 text-center">
          <div className="w-12 h-12 rounded-full bg-stone-100 dark:bg-neutral-800 flex items-center justify-center mx-auto mb-3">
            <svg
              className="w-6 h-6 text-stone-400 dark:text-neutral-500"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={1.5}>
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M21 12a2.25 2.25 0 00-2.25-2.25H15a3 3 0 11-6 0H5.25A2.25 2.25 0 003 12m18 0v6a2.25 2.25 0 01-2.25 2.25H5.25A2.25 2.25 0 013 18v-6m18 0V9M3 12V9m18-3a2.25 2.25 0 00-2.25-2.25H5.25A2.25 2.25 0 003 6m18 0V5.25A2.25 2.25 0 0018.75 3H5.25A2.25 2.25 0 003 5.25V6"
              />
            </svg>
          </div>
          <p className="text-sm text-stone-500 dark:text-neutral-400 leading-relaxed">
            {t('walletBalances.emptyState')}
          </p>
        </div>
      );
    }

    if (balances && balances.length > 0) {
      return (
        <div className="divide-y divide-stone-100 dark:divide-neutral-800">
          {balances.map(balance => (
            <BalanceRow
              key={balanceKey(balance)}
              balance={balance}
              onSend={setSendTarget}
              onReceive={setReceiveTarget}
            />
          ))}
        </div>
      );
    }

    return null;
  };

  return (
    <div>
      <div className="flex items-center justify-between pr-4">
        <SettingsHeader
          title={t('walletBalances.title')}
          showBackButton
          onBack={navigateBack}
          breadcrumbs={breadcrumbs}
        />
        <button
          type="button"
          onClick={() => void loadBalances()}
          disabled={loading}
          aria-label={t('walletBalances.refresh')}
          className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium text-primary-600 dark:text-primary-400 hover:text-primary-700 dark:hover:text-primary-300 disabled:opacity-50 transition-colors">
          <svg
            className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`}
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={2}>
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
            />
          </svg>
          {t('walletBalances.refresh')}
        </button>
      </div>

      <div className="bg-white dark:bg-neutral-900 rounded-xl border border-stone-200 dark:border-neutral-800 mx-4 mb-4 overflow-hidden">
        {renderContent()}
      </div>

      {sendTarget && (
        <SendCryptoModal
          balance={sendTarget}
          onClose={() => setSendTarget(null)}
          onSuccess={() => void loadBalances()}
        />
      )}
      {receiveTarget && (
        <ReceiveModal balance={receiveTarget} onClose={() => setReceiveTarget(null)} />
      )}
    </div>
  );
};

export default WalletBalancesPanel;
