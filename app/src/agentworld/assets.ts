/**
 * Asset resolution for Agent World / x402 surfaces.
 *
 * x402 payment challenges now return the asset as a **mint address** (e.g. the
 * USDC SPL mint) rather than a symbol like `"USDC"`. This module maps the common
 * mints back to a display symbol + decimals so the UI never shows a raw base58
 * address, and so amount scaling uses the right decimal count.
 */

/** Known Solana SPL mints → display symbol (mainnet + devnet USDC, wrapped SOL). */
const KNOWN_MINTS: Record<string, string> = {
  EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v: 'USDC', // mainnet
  '4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU': 'USDC', // devnet
  So11111111111111111111111111111111111111112: 'SOL', // wrapped SOL
};

/** Decimals per known symbol. USDC/CASH = 6, SOL/WSOL = 9, others = 0. */
export function decimalsForSymbol(symbol: string | undefined): number {
  const up = (symbol ?? '').toUpperCase();
  if (up === 'USDC' || up === 'CASH') return 6;
  if (up === 'SOL' || up === 'WSOL') return 9;
  return 0;
}

/** True when `value` looks like a base58 Solana address rather than a symbol. */
function looksLikeMint(value: string): boolean {
  return value.length >= 32 && /^[1-9A-HJ-NP-Za-km-z]+$/.test(value);
}

/**
 * Resolve an x402 asset (symbol OR mint address) to a display symbol.
 *
 * Preference order: an explicit wallet-resolved symbol → known-mint lookup →
 * the value itself when it already looks like a symbol → a truncated address.
 */
export function resolveAssetSymbol(asset: string | undefined, walletSymbol?: string): string {
  if (walletSymbol && walletSymbol.trim()) return walletSymbol.trim();
  if (!asset) return '';
  if (KNOWN_MINTS[asset]) return KNOWN_MINTS[asset];
  if (!looksLikeMint(asset)) return asset; // already a symbol
  return `${asset.slice(0, 4)}…${asset.slice(-4)}`;
}

/** Decimals for an x402 asset that may be a symbol or a mint address. */
export function decimalsForAsset(asset: string | undefined, walletDecimals?: number): number {
  if (typeof walletDecimals === 'number') return walletDecimals;
  return decimalsForSymbol(resolveAssetSymbol(asset));
}

/**
 * Format a raw base-unit integer string to a decimal string with `decimals`.
 *
 * e.g. `formatUnits("30000000", 6)` → `"30"`, `formatUnits("10500000", 6)` →
 * `"10.5"`. Pure string math (no float) so large amounts never lose precision.
 *
 * Lives here (not in X402ConfirmDialog) because both the confirm dialog AND the
 * marketplace price/reward formatters need it; X402ConfirmDialog re-exports it so
 * its existing public API is unchanged.
 */
export function formatUnits(raw: string, decimals: number): string {
  if (decimals <= 0) return raw;
  const negative = raw.startsWith('-');
  const digits = (negative ? raw.slice(1) : raw).padStart(decimals + 1, '0');
  const whole = digits.slice(0, digits.length - decimals);
  const frac = digits.slice(digits.length - decimals).replace(/0+$/, '');
  const body = frac ? `${whole}.${frac}` : whole;
  return negative ? `-${body}` : body;
}

/** Group the integer part of a numeric amount with thousands separators. */
function groupThousands(amount: string): string {
  if (!Number.isFinite(Number(amount))) return amount;
  const negative = amount.startsWith('-');
  const body = negative ? amount.slice(1) : amount;
  const [intPart, fracPart] = body.split('.');
  const grouped = Number(intPart).toLocaleString('en-US');
  const out = fracPart != null ? `${grouped}.${fracPart}` : grouped;
  return negative ? `-${out}` : out;
}

/**
 * Format a marketplace / x402 amount that is stored in the asset's smallest BASE
 * units (e.g. a 30 USDC price is stored as `"30000000"`) into a human-readable
 * string like `"30 USDC"`.
 *
 * The asset's decimals are resolved from its symbol/mint via {@link decimalsForAsset}
 * (USDC → 6) because marketplace price objects carry no `decimals` field — we
 * assume the conventional decimals for the known symbol and default to 0 for
 * unknown assets (rendered verbatim, since we can't safely humanize them).
 *
 * Display unit MUST match the human-decimal INPUT unit used by AmountCommitDialog
 * — both go through the same decimals, so a price shown as "30 USDC" is the value
 * a user types when bidding/offering.
 */
export function formatAssetAmount(amount: string, asset: string): string {
  const decimals = decimalsForAsset(asset);
  const display = decimals > 0 ? formatUnits(amount, decimals) : amount;
  return `${groupThousands(display)} ${resolveAssetSymbol(asset)}`;
}
