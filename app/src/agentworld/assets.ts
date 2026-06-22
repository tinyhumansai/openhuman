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
