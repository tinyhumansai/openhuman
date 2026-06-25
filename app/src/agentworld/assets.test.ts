/**
 * Tests for Agent World asset helpers — symbol/decimals resolution and the
 * base-unit → human amount formatter shared by marketplace prices, bounty
 * rewards, and the x402 confirm dialog.
 */
import { describe, expect, test } from 'vitest';

import { decimalsForAsset, formatAssetAmount, formatUnits, resolveAssetSymbol } from './assets';

describe('formatUnits', () => {
  test('shifts the decimal point by the asset decimals', () => {
    expect(formatUnits('30000000', 6)).toBe('30');
    expect(formatUnits('10500000', 6)).toBe('10.5');
    expect(formatUnits('1', 6)).toBe('0.000001');
  });

  test('returns the raw string unchanged for zero decimals', () => {
    expect(formatUnits('42', 0)).toBe('42');
  });
});

describe('formatAssetAmount', () => {
  test('humanizes a base-unit USDC price so display matches decimal input', () => {
    // A 30 USDC listing is stored as "30000000" base units (USDC = 6 decimals).
    // It MUST render as the human "30 USDC" — the same value a user types into
    // AmountCommitDialog — not the raw base-unit string.
    expect(formatAssetAmount('30000000', 'USDC')).toBe('30 USDC');
  });

  test('keeps the fractional part of a non-round base-unit amount', () => {
    expect(formatAssetAmount('10500000', 'USDC')).toBe('10.5 USDC');
  });

  test('groups large amounts with thousands separators', () => {
    // 1,234,000 USDC = 1234000 * 10^6 base units.
    expect(formatAssetAmount('1234000000000', 'USDC')).toBe('1,234,000 USDC');
  });

  test('resolves a USDC mint address to symbol + decimals', () => {
    // Mainnet USDC SPL mint → 30 USDC.
    expect(formatAssetAmount('30000000', 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v')).toBe(
      '30 USDC'
    );
  });

  test('renders an unknown (0-decimal) asset verbatim', () => {
    // No known decimals → no division, no double-conversion of an unknown unit.
    expect(formatAssetAmount('500', 'MYSTERY')).toBe('500 MYSTERY');
  });
});

describe('asset resolution sanity', () => {
  test('USDC resolves to 6 decimals', () => {
    expect(decimalsForAsset('USDC')).toBe(6);
    expect(resolveAssetSymbol('USDC')).toBe('USDC');
  });
});
