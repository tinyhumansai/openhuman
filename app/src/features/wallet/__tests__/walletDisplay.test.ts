import { describe, expect, it } from 'vitest';

import {
  balanceBadge,
  balanceKey,
  balanceNetworkLabel,
  fromSmallestUnit,
  toSmallestUnit,
} from '../walletDisplay';

describe('walletDisplay — labels', () => {
  it('labels EVM rows by network and others by chain', () => {
    expect(balanceNetworkLabel({ chain: 'evm', evmNetwork: 'ethereum_mainnet' })).toBe('Ethereum');
    expect(balanceNetworkLabel({ chain: 'evm', evmNetwork: 'bsc_mainnet' })).toBe('BNB Chain');
    expect(balanceNetworkLabel({ chain: 'btc' })).toBe('Bitcoin');
  });

  it('badges EVM rows by network short code', () => {
    expect(balanceBadge({ chain: 'evm', evmNetwork: 'base_mainnet' })).toBe('BASE');
    expect(balanceBadge({ chain: 'evm', evmNetwork: 'bsc_mainnet' })).toBe('BSC');
    expect(balanceBadge({ chain: 'tron' })).toBe('TRX');
  });

  it('produces a stable key incorporating the network', () => {
    expect(balanceKey({ chain: 'evm', evmNetwork: 'base_mainnet', assetSymbol: 'ETH' })).toBe(
      'evm-base_mainnet-ETH'
    );
    expect(balanceKey({ chain: 'btc', assetSymbol: 'BTC' })).toBe('btc-native-BTC');
  });
});

describe('walletDisplay — amount conversion', () => {
  it('converts human amounts to the smallest unit', () => {
    expect(toSmallestUnit('1', 18)).toBe('1000000000000000000');
    expect(toSmallestUnit('1.5', 18)).toBe('1500000000000000000');
    expect(toSmallestUnit('0.0001', 8)).toBe('10000');
    expect(toSmallestUnit('0', 6)).toBe('0');
    expect(toSmallestUnit('12', 0)).toBe('12');
  });

  it('rejects malformed amounts and excess precision', () => {
    expect(() => toSmallestUnit('', 18)).toThrow();
    expect(() => toSmallestUnit('abc', 18)).toThrow();
    expect(() => toSmallestUnit('.', 18)).toThrow();
    expect(() => toSmallestUnit('1.234', 2)).toThrow();
  });

  it('round-trips through fromSmallestUnit', () => {
    expect(fromSmallestUnit('1500000000000000000', 18)).toBe('1.5');
    expect(fromSmallestUnit('1000000000000000000', 18)).toBe('1');
    expect(fromSmallestUnit('10000', 8)).toBe('0.0001');
    expect(fromSmallestUnit('0', 9)).toBe('0');
  });
});
