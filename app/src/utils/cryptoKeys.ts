import { pbkdf2 } from '@noble/hashes/pbkdf2.js';
import { sha256 } from '@noble/hashes/sha2.js';
import { keccak_256 } from '@noble/hashes/sha3.js';
import { bytesToHex } from '@noble/hashes/utils.js';
import { getPublicKey } from '@noble/secp256k1';
import { HDKey } from '@scure/bip32';
import { generateMnemonic, mnemonicToSeedSync, validateMnemonic } from '@scure/bip39';
import { wordlist } from '@scure/bip39/wordlists/english.js';

/** Word count for newly generated recovery phrases (128-bit entropy, BIP39). */
export const MNEMONIC_GENERATE_WORD_COUNT = 12;

/**
 * Generate a 12-word BIP39 mnemonic phrase (128-bit entropy).
 */
export function generateMnemonicPhrase(): string {
  return generateMnemonic(wordlist, 128);
}

/**
 * Validate a BIP39 mnemonic phrase.
 */
export function validateMnemonicPhrase(mnemonic: string): boolean {
  return validateMnemonic(mnemonic, wordlist);
}

/**
 * Derive a 256-bit AES encryption key from a mnemonic phrase.
 * Uses BIP39 seed derivation followed by PBKDF2-SHA256.
 * Returns the key as a hex string.
 */
export function deriveAesKeyFromMnemonic(mnemonic: string): string {
  // Get the BIP39 seed (512-bit) from the mnemonic
  const seed = mnemonicToSeedSync(mnemonic);

  // Derive a 256-bit AES key using PBKDF2 with the seed
  const salt = new TextEncoder().encode('openhuman-aes-key-v1');
  const derivedKey = pbkdf2(sha256, seed, salt, { c: 100000, dkLen: 32 });

  return bytesToHex(derivedKey);
}

/** BIP44 path for first Ethereum account: m/44'/60'/0'/0/0 */
const EVM_DERIVATION_PATH = "m/44'/60'/0'/0/0";

/**
 * Derive the first EVM wallet address (Ethereum BIP44) from a mnemonic phrase.
 * Uses path m/44'/60'/0'/0/0. Returns a checksummed 0x-prefixed address.
 */
export function deriveEvmAddressFromMnemonic(mnemonic: string): string {
  const seed = mnemonicToSeedSync(mnemonic);
  const hdkey = HDKey.fromMasterSeed(seed);
  const derived = hdkey.derive(EVM_DERIVATION_PATH);
  const privateKey = derived.privateKey;
  if (!privateKey) throw new Error('Failed to derive private key');
  // Ethereum address = keccak256(uncompressed public key without 0x04)[12:]
  const pubKey = getPublicKey(privateKey, false); // uncompressed, 65 bytes
  const hash = keccak_256(pubKey.slice(1));
  const addressBytes = hash.slice(-20);
  const hex = bytesToHex(addressBytes);
  return toChecksumAddress('0x' + hex);
}

/** Simple checksum: lowercase with 0x, then capitalize by hash. */
function toChecksumAddress(address: string): string {
  const a = address.replace(/^0x/i, '').toLowerCase();
  const hash = bytesToHex(keccak_256(new TextEncoder().encode(a)));
  let result = '0x';
  for (let i = 0; i < 40; i++) {
    result += parseInt(hash[i], 16) >= 8 ? a[i].toUpperCase() : a[i];
  }
  return result;
}
