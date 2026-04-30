import { describe, expect, it } from 'vitest';

import {
  deriveAesKeyFromMnemonic,
  deriveEvmAddressFromMnemonic,
  generateMnemonicPhrase,
  MNEMONIC_GENERATE_WORD_COUNT,
  validateMnemonicPhrase,
} from './cryptoKeys';

// Known-good 12-word BIP39 mnemonic for deterministic assertions.
const KNOWN_MNEMONIC =
  'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';

describe('MNEMONIC_GENERATE_WORD_COUNT', () => {
  it('is 12', () => {
    expect(MNEMONIC_GENERATE_WORD_COUNT).toBe(12);
  });
});

describe('generateMnemonicPhrase', () => {
  it('returns a 12-word phrase', () => {
    const phrase = generateMnemonicPhrase();
    expect(phrase.trim().split(/\s+/)).toHaveLength(12);
  });

  it('returns a valid BIP39 mnemonic', () => {
    const phrase = generateMnemonicPhrase();
    expect(validateMnemonicPhrase(phrase)).toBe(true);
  });

  it('produces a different phrase each call', () => {
    const a = generateMnemonicPhrase();
    const b = generateMnemonicPhrase();
    // Astronomically unlikely to collide; guards against a no-op implementation.
    expect(a).not.toBe(b);
  });
});

describe('validateMnemonicPhrase', () => {
  it('returns true for the known valid mnemonic', () => {
    expect(validateMnemonicPhrase(KNOWN_MNEMONIC)).toBe(true);
  });

  it('returns false for an empty string', () => {
    expect(validateMnemonicPhrase('')).toBe(false);
  });

  it('returns false for a single random word', () => {
    expect(validateMnemonicPhrase('abandon')).toBe(false);
  });

  it('returns false for 12 non-BIP39 words', () => {
    expect(
      validateMnemonicPhrase('foo bar baz qux quux corge grault garply waldo fred plugh xyzzy')
    ).toBe(false);
  });

  it('returns false for an otherwise valid phrase with one word replaced by a non-wordlist word', () => {
    const tampered = KNOWN_MNEMONIC.replace('abandon', 'xxxxxxxx');
    expect(validateMnemonicPhrase(tampered)).toBe(false);
  });
});

describe('deriveAesKeyFromMnemonic', () => {
  it('returns a 64-character hex string (256-bit key)', () => {
    const key = deriveAesKeyFromMnemonic(KNOWN_MNEMONIC);
    expect(key).toMatch(/^[0-9a-f]{64}$/);
  });

  it('is deterministic for the same mnemonic', () => {
    const key1 = deriveAesKeyFromMnemonic(KNOWN_MNEMONIC);
    const key2 = deriveAesKeyFromMnemonic(KNOWN_MNEMONIC);
    expect(key1).toBe(key2);
  });

  it('produces a different key for a different mnemonic', () => {
    const other = 'zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong';
    expect(validateMnemonicPhrase(other)).toBe(true);
    const keyA = deriveAesKeyFromMnemonic(KNOWN_MNEMONIC);
    const keyB = deriveAesKeyFromMnemonic(other);
    expect(keyA).not.toBe(keyB);
  });

  it('returns a fixed known value for the all-abandon mnemonic', () => {
    // Pinned output for salt='openhuman-aes-key-v1', PBKDF2-SHA256, c=100000, dkLen=32.
    // If this assertion fails, the KDF parameters or salt have changed — update intentionally.
    const key = deriveAesKeyFromMnemonic(KNOWN_MNEMONIC);
    expect(key).toBe('dce707ee483afb0a70cb2e076295f9f914e0c62cc097895eabda1c0c1f2f0cb1');
  });
});

describe('deriveEvmAddressFromMnemonic', () => {
  it('returns a 0x-prefixed 42-character string', () => {
    const address = deriveEvmAddressFromMnemonic(KNOWN_MNEMONIC);
    expect(address).toMatch(/^0x[0-9a-fA-F]{40}$/);
  });

  it('is deterministic for the same mnemonic', () => {
    const addr1 = deriveEvmAddressFromMnemonic(KNOWN_MNEMONIC);
    const addr2 = deriveEvmAddressFromMnemonic(KNOWN_MNEMONIC);
    expect(addr1).toBe(addr2);
  });

  it('returns the well-known address for the all-abandon mnemonic', () => {
    // MetaMask / BIP44 m/44'/60'/0'/0/0 for all-abandon is a stable known address.
    // Pinned in EIP-55 checksummed form — validates both identity and checksum casing.
    const address = deriveEvmAddressFromMnemonic(KNOWN_MNEMONIC);
    expect(address).toBe('0x9858EfFD232B4033E47d90003D41EC34EcaEda94');
  });

  it('returns a different address for a different mnemonic', () => {
    const other = 'zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong';
    const addrA = deriveEvmAddressFromMnemonic(KNOWN_MNEMONIC);
    const addrB = deriveEvmAddressFromMnemonic(other);
    expect(addrA).not.toBe(addrB);
  });

  it('produces a checksummed address (EIP-55)', () => {
    const address = deriveEvmAddressFromMnemonic(KNOWN_MNEMONIC);
    // At least some characters must be uppercase (otherwise it would just be lowercase hex).
    // EIP-55 checksummed addresses contain mixed case by design.
    const hex = address.slice(2);
    // Not all-lowercase and not all-uppercase → mixed casing applied.
    const hasUpper = hex !== hex.toLowerCase();
    const hasLower = hex !== hex.toUpperCase();
    expect(hasUpper && hasLower).toBe(true);
  });
});
