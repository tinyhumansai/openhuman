import { pbkdf2 } from '@noble/hashes/pbkdf2.js';
import { sha256 } from '@noble/hashes/sha2.js';
import { bytesToHex } from '@noble/hashes/utils.js';

/** Word count used when the wallet module generates a new recovery phrase. */
export const MNEMONIC_GENERATE_WORD_COUNT = 12;

/**
 * Derive the existing 256-bit OpenHuman storage key from a module-derived
 * BIP-39 seed. The seed buffer is cleared after the generic storage KDF runs.
 */
export function deriveAesKeyFromSeed(seed: Uint8Array): string {
  if (seed.length !== 64) {
    seed.fill(0);
    throw new Error('Recovery seed must be exactly 64 bytes.');
  }
  const salt = new TextEncoder().encode('openhuman-aes-key-v1');
  try {
    const derivedKey = pbkdf2(sha256, seed, salt, { c: 100000, dkLen: 32 });
    return bytesToHex(derivedKey);
  } finally {
    seed.fill(0);
  }
}
