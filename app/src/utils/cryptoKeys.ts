import { ed25519 } from '@noble/curves/ed25519.js';
import { hmac } from '@noble/hashes/hmac.js';
import { ripemd160 } from '@noble/hashes/legacy.js';
import { pbkdf2 } from '@noble/hashes/pbkdf2.js';
import { sha256, sha512 } from '@noble/hashes/sha2.js';
import { keccak_256 } from '@noble/hashes/sha3.js';
import { bytesToHex } from '@noble/hashes/utils.js';
import { getPublicKey } from '@noble/secp256k1';
import { base58 } from '@scure/base';
import { HDKey } from '@scure/bip32';
import { generateMnemonic, mnemonicToSeedSync, validateMnemonic } from '@scure/bip39';
import { wordlist } from '@scure/bip39/wordlists/english.js';

/** Word count for newly generated recovery phrases (128-bit entropy, BIP39). */
export const MNEMONIC_GENERATE_WORD_COUNT = 12;
type WalletChain = 'evm' | 'btc' | 'solana' | 'tron';
export type WalletSetupSource = 'generated' | 'imported';

interface WalletAccountIdentity {
  chain: WalletChain;
  address: string;
  derivationPath: string;
}

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
const BTC_DERIVATION_PATH = "m/44'/0'/0'/0/0";
const SOLANA_DERIVATION_PATH = "m/44'/501'/0'/0'";
const TRON_DERIVATION_PATH = "m/44'/195'/0'/0/0";

/**
 * Derive the first EVM wallet address (Ethereum BIP44) from a mnemonic phrase.
 * Uses path m/44'/60'/0'/0/0. Returns a checksummed 0x-prefixed address.
 */
export function deriveEvmAddressFromMnemonic(mnemonic: string): string {
  const privateKey = deriveSecp256k1PrivateKey(mnemonic, EVM_DERIVATION_PATH);
  // Ethereum address = keccak256(uncompressed public key without 0x04)[12:]
  const pubKey = getPublicKey(privateKey, false); // uncompressed, 65 bytes
  const hash = keccak_256(pubKey.slice(1));
  const addressBytes = hash.slice(-20);
  const hex = bytesToHex(addressBytes);
  return toChecksumAddress('0x' + hex);
}

export function deriveBtcAddressFromMnemonic(mnemonic: string): string {
  const privateKey = deriveSecp256k1PrivateKey(mnemonic, BTC_DERIVATION_PATH);
  const compressedPublicKey = getPublicKey(privateKey, true);
  const payload = new Uint8Array(21);
  payload[0] = 0x00;
  payload.set(ripemd160(sha256(compressedPublicKey)), 1);
  return base58CheckEncode(payload);
}

export function deriveSolanaAddressFromMnemonic(mnemonic: string): string {
  const seed = mnemonicToSeedSync(mnemonic);
  const privateKey = deriveSlip10Ed25519PrivateKey(seed, SOLANA_DERIVATION_PATH);
  return base58.encode(ed25519.getPublicKey(privateKey));
}

export function deriveTronAddressFromMnemonic(mnemonic: string): string {
  const privateKey = deriveSecp256k1PrivateKey(mnemonic, TRON_DERIVATION_PATH);
  const publicKey = getPublicKey(privateKey, false);
  const payload = new Uint8Array(21);
  payload[0] = 0x41;
  payload.set(keccak_256(publicKey.slice(1)).slice(-20), 1);
  return base58CheckEncode(payload);
}

export function deriveWalletAccountsFromMnemonic(mnemonic: string): WalletAccountIdentity[] {
  return [
    {
      chain: 'evm',
      address: deriveEvmAddressFromMnemonic(mnemonic),
      derivationPath: EVM_DERIVATION_PATH,
    },
    {
      chain: 'btc',
      address: deriveBtcAddressFromMnemonic(mnemonic),
      derivationPath: BTC_DERIVATION_PATH,
    },
    {
      chain: 'solana',
      address: deriveSolanaAddressFromMnemonic(mnemonic),
      derivationPath: SOLANA_DERIVATION_PATH,
    },
    {
      chain: 'tron',
      address: deriveTronAddressFromMnemonic(mnemonic),
      derivationPath: TRON_DERIVATION_PATH,
    },
  ];
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

function deriveSecp256k1PrivateKey(mnemonic: string, derivationPath: string): Uint8Array {
  const seed = mnemonicToSeedSync(mnemonic);
  const hdkey = HDKey.fromMasterSeed(seed);
  const derived = hdkey.derive(derivationPath);
  if (!derived.privateKey) {
    throw new Error(`Failed to derive private key for path ${derivationPath}`);
  }
  return derived.privateKey;
}

function deriveSlip10Ed25519PrivateKey(seed: Uint8Array, derivationPath: string): Uint8Array {
  let key = hmac(sha512, new TextEncoder().encode('ed25519 seed'), seed);
  let privateKey = key.slice(0, 32);
  let chainCode = key.slice(32);

  for (const segment of derivationPath.split('/').slice(1)) {
    if (!segment.endsWith("'")) {
      throw new Error(`Ed25519 derivation path must be fully hardened: ${derivationPath}`);
    }
    const index = Number.parseInt(segment.slice(0, -1), 10);
    const hardened = (index + 0x80000000) >>> 0;
    const data = new Uint8Array(37);
    data[0] = 0;
    data.set(privateKey, 1);
    data[33] = (hardened >>> 24) & 0xff;
    data[34] = (hardened >>> 16) & 0xff;
    data[35] = (hardened >>> 8) & 0xff;
    data[36] = hardened & 0xff;
    key = hmac(sha512, chainCode, data);
    privateKey = key.slice(0, 32);
    chainCode = key.slice(32);
  }

  return privateKey;
}

function base58CheckEncode(payload: Uint8Array): string {
  const checksum = sha256(sha256(payload)).slice(0, 4);
  const bytes = new Uint8Array(payload.length + checksum.length);
  bytes.set(payload, 0);
  bytes.set(checksum, payload.length);
  return base58.encode(bytes);
}
