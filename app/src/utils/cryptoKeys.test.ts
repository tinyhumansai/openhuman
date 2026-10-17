import { describe, expect, it } from 'vitest';

import { deriveAesKeyFromSeed } from './cryptoKeys';

// BIP-39's published empty-passphrase seed for `abandon` x11 + `about`.
const CANONICAL_SEED_HEX =
  '5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc19a5ac40b389cd370d086206dec8aa6c43daea6690f20ad3d8d48b2d2ce9e38e4';

function bytesFromHex(hex: string): Uint8Array {
  return Uint8Array.from(hex.match(/.{2}/g) ?? [], byte => Number.parseInt(byte, 16));
}

describe('deriveAesKeyFromSeed', () => {
  it('preserves the existing storage-key result for the canonical BIP-39 seed', () => {
    const seed = bytesFromHex(CANONICAL_SEED_HEX);

    expect(deriveAesKeyFromSeed(seed)).toBe(
      'dce707ee483afb0a70cb2e076295f9f914e0c62cc097895eabda1c0c1f2f0cb1'
    );
    expect(seed).toEqual(new Uint8Array(64));
  });

  it('rejects a non-BIP-39-sized seed and clears it', () => {
    const seed = new Uint8Array([1, 2, 3]);

    expect(() => deriveAesKeyFromSeed(seed)).toThrow(/exactly 64 bytes/i);
    expect(seed).toEqual(new Uint8Array(3));
  });
});
