/**
 * X402ConfirmDialog — confirm-before-spend dialog for Agent World x402 flows.
 *
 * Reused by every write flow that moves funds (register / buy / bid / offer):
 * it shows the payment amount, the asset, the wallet's balance and address, and
 * gates the "Confirm & Pay" button on having enough balance. The parent owns the
 * actual payment call — this component only renders the confirmation and reports
 * the user's decision via `onConfirm` / `onCancel`.
 *
 * Money only moves after the user clicks Confirm (which the parent wires to the
 * `confirmed: true` RPC) — this dialog never calls the backend itself.
 */
import Button from '../../components/ui/Button';
import { ModalShell } from '../../components/ui/ModalShell';
import { openUrl } from '../../utils/openUrl';
import { decimalsForAsset, resolveAssetSymbol } from '../assets';

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

export interface X402ConfirmDialogProps {
  /** Title shown in the modal header (e.g. "Register @handle"). */
  title: string;
  /** Optional subtitle / context line. */
  subtitle?: string;
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
  onConfirm: () => void;
  onCancel: () => void;
}

/** Format a raw base-unit integer string to a decimal string with `decimals`. */
export function formatUnits(raw: string, decimals: number): string {
  if (decimals <= 0) return raw;
  const negative = raw.startsWith('-');
  const digits = (negative ? raw.slice(1) : raw).padStart(decimals + 1, '0');
  const whole = digits.slice(0, digits.length - decimals);
  const frac = digits.slice(digits.length - decimals).replace(/0+$/, '');
  const body = frac ? `${whole}.${frac}` : whole;
  return negative ? `-${body}` : body;
}

/** True when the wallet provably cannot cover `amount`. Unknown balance → false. */
export function isInsufficient(balance: X402WalletBalance | null, amount: string): boolean {
  if (!balance) return false;
  try {
    return BigInt(balance.raw) < BigInt(amount);
  } catch {
    return false;
  }
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
  amount,
  asset,
  network,
  balance,
  walletAddress,
  busy = false,
  busyLabel = 'Processing…',
  onConfirm,
  onCancel,
}: X402ConfirmDialogProps) {
  // `asset` may arrive as a mint address; resolve to a display symbol + decimals
  // (preferring the wallet's own resolution when present).
  const assetSymbol = resolveAssetSymbol(asset, balance?.assetSymbol);
  const decimals = decimalsForAsset(asset, balance?.decimals);
  const amountDisplay = formatUnits(amount, decimals);
  const insufficient = isInsufficient(balance, amount);
  const confirmDisabled = busy || insufficient;

  return (
    <ModalShell
      title={title}
      titleId="x402-confirm-title"
      subtitle={subtitle}
      onClose={busy ? () => undefined : onCancel}
      maxWidthClassName="max-w-sm">
      <div className="space-y-4">
        <div className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-900/50 p-4 space-y-3">
          <Row label="Amount">
            <span
              className="font-semibold text-stone-900 dark:text-neutral-100"
              data-testid="x402-amount">
              {amountDisplay} {assetSymbol}
            </span>
          </Row>
          <Row label="Network">
            <span className="text-xs text-stone-500 dark:text-neutral-400">
              {friendlyNetwork(network)}
            </span>
          </Row>
          <Row label="Your balance">
            <span
              className={`font-medium ${
                insufficient ? 'text-coral-500' : 'text-stone-700 dark:text-neutral-200'
              }`}
              data-testid="x402-balance">
              {balance ? `${balance.formatted} ${balance.assetSymbol}` : 'Unknown'}
            </span>
          </Row>
          <Row label="Wallet">
            <span className="font-mono text-xs text-stone-500 dark:text-neutral-400">
              {truncateAddress(walletAddress)}
            </span>
          </Row>
        </div>

        {insufficient ? (
          <p className="text-xs text-coral-500" data-testid="x402-insufficient">
            Insufficient {assetSymbol} balance to complete this payment. Add funds to your wallet to
            continue.
          </p>
        ) : (
          <p className="text-xs text-stone-400 dark:text-neutral-500">
            Your wallet will sign and broadcast this payment on {friendlyNetwork(network)}.
          </p>
        )}

        <div className="flex justify-end gap-2">
          <Button variant="secondary" size="sm" onClick={onCancel} disabled={busy}>
            Cancel
          </Button>
          {insufficient ? (
            // Not enough balance — send the user to the tiny.place fund page for
            // the exact wallet + asset instead of a dead, disabled Pay button.
            <Button
              variant="primary"
              size="sm"
              onClick={() => {
                void openUrl(fundingUrl(walletAddress, assetSymbol));
              }}
              data-testid="x402-add-funds">
              Add funds
            </Button>
          ) : (
            <Button
              variant="primary"
              size="sm"
              onClick={onConfirm}
              disabled={confirmDisabled}
              data-testid="x402-confirm">
              {busy ? busyLabel : 'Confirm & Pay'}
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
      <span className="text-xs text-stone-400 dark:text-neutral-500">{label}</span>
      {children}
    </div>
  );
}
