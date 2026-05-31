// Display helpers for the wallet balances surface: human-readable EVM network
// labels and lossless conversion between human-entered amounts and the chain's
// smallest unit (wei / sat / lamport / sun) using BigInt (no float rounding).
//
// Chain / network proper names (Ethereum, Base, BNB Chain, …) are brand names
// rendered verbatim, so they intentionally bypass i18n.
import type { BalanceInfo, EvmNetwork, WalletChain } from '../../services/walletApi';

/** Full display name per EVM network. */
export const EVM_NETWORK_LABEL: Record<EvmNetwork, string> = {
  ethereum_mainnet: 'Ethereum',
  base_mainnet: 'Base',
  arbitrum_one: 'Arbitrum',
  optimism_mainnet: 'Optimism',
  polygon_mainnet: 'Polygon',
  bsc_mainnet: 'BNB Chain',
};

/** Short badge label per EVM network. */
export const EVM_NETWORK_BADGE: Record<EvmNetwork, string> = {
  ethereum_mainnet: 'ETH',
  base_mainnet: 'BASE',
  arbitrum_one: 'ARB',
  optimism_mainnet: 'OP',
  polygon_mainnet: 'POL',
  bsc_mainnet: 'BSC',
};

const CHAIN_LABEL: Record<WalletChain, string> = {
  evm: 'EVM',
  btc: 'Bitcoin',
  solana: 'Solana',
  tron: 'Tron',
};

/** Human network/chain label for a balance row (network name for EVM rows). */
export function balanceNetworkLabel(balance: Pick<BalanceInfo, 'chain' | 'evmNetwork'>): string {
  if (balance.chain === 'evm' && balance.evmNetwork) {
    return EVM_NETWORK_LABEL[balance.evmNetwork] ?? 'EVM';
  }
  return CHAIN_LABEL[balance.chain] ?? balance.chain.toUpperCase();
}

/** Short badge text for a balance row. */
export function balanceBadge(balance: Pick<BalanceInfo, 'chain' | 'evmNetwork'>): string {
  if (balance.chain === 'evm' && balance.evmNetwork) {
    return EVM_NETWORK_BADGE[balance.evmNetwork] ?? 'EVM';
  }
  return { evm: 'EVM', btc: 'BTC', solana: 'SOL', tron: 'TRX' }[balance.chain];
}

/** Stable React key for a balance row (chain + network + symbol). */
export function balanceKey(
  balance: Pick<BalanceInfo, 'chain' | 'evmNetwork' | 'assetSymbol'>
): string {
  return `${balance.chain}-${balance.evmNetwork ?? 'native'}-${balance.assetSymbol}`;
}

/**
 * Convert a human-entered decimal amount (e.g. "1.5") into the asset's smallest
 * unit as a decimal string (e.g. "1500000000000000000" for 18 decimals).
 * Throws on malformed input or more fractional digits than `decimals` allows.
 *
 * The thrown messages are internal sentinels (developer-facing only): callers
 * catch them and surface a translated, user-facing message via `useT()` —
 * never render `error.message` from here directly.
 */
export function toSmallestUnit(human: string, decimals: number): string {
  const trimmed = human.trim();
  if (trimmed === '' || trimmed === '.' || !/^\d*\.?\d*$/.test(trimmed)) {
    throw new Error('invalid_amount');
  }
  const [whole, frac = ''] = trimmed.split('.');
  if (frac.length > decimals) {
    throw new Error('too_many_decimals');
  }
  const combined = `${whole || '0'}${frac.padEnd(decimals, '0')}`;
  const normalized = combined.replace(/^0+(?=\d)/, '');
  return normalized === '' ? '0' : normalized;
}

/**
 * Format a smallest-unit decimal string back to a human amount, trimming
 * trailing fractional zeros. Inverse of {@link toSmallestUnit}.
 */
export function fromSmallestUnit(raw: string, decimals: number): string {
  if (!/^\d+$/.test(raw)) return raw;
  if (decimals === 0) return raw;
  const padded = raw.padStart(decimals + 1, '0');
  const whole = padded.slice(0, padded.length - decimals);
  const frac = padded.slice(padded.length - decimals).replace(/0+$/, '');
  return frac.length > 0 ? `${whole}.${frac}` : whole;
}
