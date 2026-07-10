/**
 * X402ConfirmDialog — confirm-before-spend dialog for Agent World x402 flows.
 *
 * Reused by every write flow that involves funds (register / buy / bid / offer)
 * across two modes:
 *
 * - `mode="spend"` (register / buy): an IMMEDIATE on-chain spend. A provably
 *   insufficient balance HARD-gates the Pay button (we replace it with an
 *   "Add funds" redirect) so we never broadcast a payment that must fail.
 * - `mode="commit"` (bid / offer): a SIGNED COMMITMENT that only settles if the
 *   counterparty accepts it later — there is no transfer at submit time. A low
 *   balance is therefore a SOFT warning, not a block: we surface it but still
 *   allow Confirm.
 *
 * In BOTH modes an UNKNOWN balance (couldn't be fetched) is never treated as
 * "sufficient" — we show an explicit "couldn't verify balance" note and allow
 * Confirm (the backend remains the authoritative gate), instead of silently
 * pretending the wallet can cover the amount.
 *
 * The parent owns the actual payment / commitment call — this component only
 * renders the confirmation and reports the user's decision via `onConfirm` /
 * `onCancel`. Money only moves after the user clicks Confirm.
 */
import Button from '../../components/ui/Button';
import { ModalShell } from '../../components/ui/ModalShell';
import { useT } from '../../lib/i18n/I18nContext';
import { openUrl } from '../../utils/openUrl';
import { decimalsForAsset, formatUnits, resolveAssetSymbol } from '../assets';

// Re-exported from `../assets` so existing importers (LedgerSection, BountiesSection,
// ExploreSection, tests) keep importing `formatUnits` from this module unchanged.
// The implementation lives in `assets.ts` because the marketplace price formatter
// needs it too, and `assets.ts` cannot import from here without a cycle.
export { formatUnits };

/** tiny.place hosted funding page — handles deposits / on-ramp for the wallet. */
const FUND_PAGE_URL = 'https://tiny.place/fund';

/**
 * Build the tiny.place funding URL for a wallet + asset, e.g.
 * `https://tiny.place/fund?address=<addr>&asset=USDC`. The fund page reads
 * these params to pre-fill the deposit target, so the user lands ready to top
 * up the exact wallet that came up short.
 */
export function fundingUrl(address: string, asset: string): string {
  const params = new URLSearchParams({ address, asset });
  return `${FUND_PAGE_URL}?${params.toString()}`;
}

export interface X402WalletBalance {
  /** Balance in raw base units (same scale as the challenge amount). */
  raw: string;
  /** Human-formatted balance (e.g. "12.50"). */
  formatted: string;
  /** Decimals for the asset (USDC = 6). */
  decimals: number;
  assetSymbol: string;
}

/**
 * Which kind of x402 write this dialog is confirming:
 * - `spend`  — immediate on-chain payment (register / buy). Insufficient balance
 *   HARD-blocks (Pay → "Add funds").
 * - `commit` — signed bid/offer commitment. Insufficient balance SOFT-warns but
 *   still allows Confirm (funds move only on acceptance).
 */
export type X402ConfirmMode = 'spend' | 'commit';

export interface X402ConfirmDialogProps {
  /** Title shown in the modal header (e.g. "Register @handle"). */
  title: string;
  /** Optional subtitle / context line. */
  subtitle?: string;
  /**
   * Confirmation mode. Defaults to `spend` (the original immediate-payment
   * behaviour) so existing register/buy call sites are unchanged.
   */
  mode?: X402ConfirmMode;
  /** Payment amount in raw base units (from the x402 challenge). */
  amount: string;
  /** Asset symbol, e.g. "USDC". */
  asset: string;
  /** Network label (e.g. "solana-devnet"), shown for transparency. */
  network?: string;
  /** The wallet's balance for `asset`, or null if it couldn't be fetched. */
  balance: X402WalletBalance | null;
  /** The paying wallet address. */
  walletAddress: string;
  /** When true, the confirm button shows `busyLabel` and is disabled. */
  busy?: boolean;
  /** Label shown on the confirm button while `busy` (e.g. "Broadcasting…"). */
  busyLabel?: string;
  /** Override the confirm-button label (e.g. "Confirm bid" in commit mode). */
  confirmLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * Three-way balance assessment against an amount:
 * - `unknown`      — balance is null/unparseable (couldn't verify). NEVER treated
 *                    as sufficient; the dialog surfaces this distinctly.
 * - `insufficient` — balance is provably below `amount`.
 * - `sufficient`   — balance provably covers `amount`.
 *
 * This replaces the old boolean-only check whose `null → false` collapsed the
 * "unknown" case into "sufficient", silently bypassing the gate. Callers that
 * only need the provable-shortfall signal can use `isInsufficient` below.
 */
export type BalanceStatus = 'unknown' | 'insufficient' | 'sufficient';

export function balanceStatus(balance: X402WalletBalance | null, amount: string): BalanceStatus {
  if (!balance) return 'unknown';
  try {
    return BigInt(balance.raw) < BigInt(amount) ? 'insufficient' : 'sufficient';
  } catch {
    // A balance row we can't parse is not a verified balance.
    return 'unknown';
  }
}

/**
 * True ONLY when the wallet provably cannot cover `amount`. Unknown balance →
 * false (the shortfall is not proven). Retained for callers that want just the
 * hard-block signal; prefer `balanceStatus` when the unknown case matters.
 */
export function isInsufficient(balance: X402WalletBalance | null, amount: string): boolean {
  return balanceStatus(balance, amount) === 'insufficient';
}

function truncateAddress(addr: string): string {
  if (!addr) return '—';
  if (addr.length <= 12) return addr;
  return `${addr.slice(0, 6)}…${addr.slice(-4)}`;
}

/**
 * A human-readable network label. tiny.place reports the CAIP-2 Solana network
 * as the raw mainnet genesis hash (`solana:5eykt4…`) on every cluster, which is
 * meaningless to users and overflows the row — so collapse any Solana network to
 * a friendly "Solana" (or "Solana (devnet)" when the label explicitly says so).
 */
export function friendlyNetwork(network?: string): string {
  if (!network) return 'Solana';
  const n = network.toLowerCase();
  if (n.includes('devnet')) return 'Solana (devnet)';
  if (n.startsWith('solana') || n.includes('5eykt4')) return 'Solana';
  return network;
}

export default function X402ConfirmDialog({
  title,
  subtitle,
  mode = 'spend',
  amount,
  asset,
  network,
  balance,
  walletAddress,
  busy = false,
  busyLabel = 'Processing…',
  confirmLabel,
  onConfirm,
  onCancel,
}: X402ConfirmDialogProps) {
  const { t } = useT();
  // `asset` may arrive as a mint address; resolve to a display symbol + decimals
  // (preferring the wallet's own resolution when present).
  const assetSymbol = resolveAssetSymbol(asset, balance?.assetSymbol);
  const decimals = decimalsForAsset(asset, balance?.decimals);
  const amountDisplay = formatUnits(amount, decimals);
  const status = balanceStatus(balance, amount);
  const insufficient = status === 'insufficient';
  const unknownBalance = status === 'unknown';

  // HARD block (replace Pay with Add-funds) applies ONLY to immediate spends with
  // a PROVEN shortfall. Commitments never hard-block; unknown balance never
  // hard-blocks (we couldn't prove a shortfall — surface a note instead).
  const hardBlock = mode === 'spend' && insufficient;
  // SOFT warning: a proven shortfall on a commitment (allowed, but flagged).
  const softWarn = mode === 'commit' && insufficient;
  const confirmDisabled = busy || hardBlock;

  console.debug('[agentworld:x402-confirm] render', {
    mode,
    status,
    hardBlock,
    softWarn,
    unknownBalance,
    busy,
  });

  const defaultConfirmLabel =
    mode === 'commit'
      ? t('agentWorld.trading.confirmCommit', 'Confirm')
      : t('agentWorld.trading.confirmPay', 'Confirm & Pay');

  return (
    <ModalShell
      title={title}
      titleId="x402-confirm-title"
      subtitle={subtitle}
      onClose={busy ? () => undefined : onCancel}
      maxWidthClassName="max-w-sm">
      <div className="space-y-4">
        <div className="rounded-lg border border-line bg-surface-muted p-4 space-y-3">
          <Row label={t('agentWorld.trading.amountLabel', 'Amount')}>
            <span className="font-semibold text-content" data-testid="x402-amount">
              {amountDisplay} {assetSymbol}
            </span>
          </Row>
          <Row label={t('agentWorld.trading.networkLabel', 'Network')}>
            <span className="text-xs text-content-muted">{friendlyNetwork(network)}</span>
          </Row>
          <Row label={t('agentWorld.trading.balanceLabel', 'Your balance')}>
            <span
              className={`font-medium ${
                insufficient ? 'text-coral-500' : 'text-content-secondary'
              }`}
              data-testid="x402-balance">
              {balance
                ? `${balance.formatted} ${balance.assetSymbol}`
                : t('agentWorld.trading.balanceUnknown', 'Unknown')}
            </span>
          </Row>
          <Row label={t('agentWorld.trading.walletLabel', 'Wallet')}>
            <span className="font-mono text-xs text-content-muted">
              {truncateAddress(walletAddress)}
            </span>
          </Row>
        </div>

        {hardBlock ? (
          <p className="text-xs text-coral-500" data-testid="x402-insufficient">
            {t(
              'agentWorld.trading.spendInsufficient',
              `Insufficient ${assetSymbol} balance to complete this payment. Add funds to your wallet to continue.`
            )}
          </p>
        ) : softWarn ? (
          // Commitment with a proven shortfall — allowed, but flag it so the user
          // knows the wallet may not cover it if the commitment is accepted.
          <p className="text-xs text-amber-500" data-testid="x402-commit-warning">
            {t(
              'agentWorld.trading.commitInsufficientWarning',
              `Your ${assetSymbol} balance may not cover this if the commitment is accepted. You can still submit it — funds only move on acceptance.`
            )}
          </p>
        ) : unknownBalance ? (
          // Balance couldn't be verified. Do NOT pretend it's sufficient — say so,
          // and let the user proceed (the backend is the authoritative gate).
          <p className="text-xs text-amber-500" data-testid="x402-balance-unverified">
            {t(
              'agentWorld.trading.balanceUnverified',
              "We couldn't verify your wallet balance. You can still continue — the payment is checked when it is submitted."
            )}
          </p>
        ) : mode === 'commit' ? (
          <p className="text-xs text-content-faint">
            {t(
              'agentWorld.trading.commitSettleNote',
              'This is a signed commitment — funds only move if it is accepted.'
            )}
          </p>
        ) : (
          <p className="text-xs text-content-faint">
            {t(
              'agentWorld.trading.spendBroadcastNote',
              'Your wallet will sign and broadcast this payment on'
            )}{' '}
            {friendlyNetwork(network)}.
          </p>
        )}

        <div className="flex justify-end gap-2">
          <Button variant="secondary" size="sm" onClick={onCancel} disabled={busy}>
            {t('agentWorld.trading.cancel', 'Cancel')}
          </Button>
          {hardBlock ? (
            // Not enough balance for an immediate spend — send the user to the
            // tiny.place fund page for the exact wallet + asset instead of a dead,
            // disabled Pay button.
            <Button
              variant="primary"
              size="sm"
              onClick={() => {
                void openUrl(fundingUrl(walletAddress, assetSymbol));
              }}
              data-testid="x402-add-funds">
              {t('agentWorld.trading.addFunds', 'Add funds')}
            </Button>
          ) : (
            <Button
              variant="primary"
              size="sm"
              onClick={onConfirm}
              disabled={confirmDisabled}
              data-testid="x402-confirm">
              {busy ? busyLabel : (confirmLabel ?? defaultConfirmLabel)}
            </Button>
          )}
        </div>
      </div>
    </ModalShell>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="text-xs text-content-faint">{label}</span>
      {children}
    </div>
  );
}
