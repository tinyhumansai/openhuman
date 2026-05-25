/**
 * Tunnel crypto: X25519 key agreement + XChaCha20-Poly1305 frame encryption.
 *
 * Wire format (encrypted frame):
 *   version(1=0x01) || nonce(24) || ciphertext+tag
 *
 * Sealed-handshake format (device → core, first frame):
 *   0x01 || eph_pub(32) || nonce(24) || ciphertext+tag
 *
 * Mirrors src/openhuman/devices/crypto.rs — keep in sync.
 */
import { xchacha20poly1305 } from '@noble/ciphers/chacha';
import { randomBytes } from '@noble/ciphers/webcrypto';
import { x25519 } from '@noble/curves/ed25519.js';
import debug from 'debug';

const cryptoLog = debug('crypto');
const cryptoErr = debug('crypto:error');

// -- constants ---------------------------------------------------------------

const FRAME_VERSION = 0x01;
const NONCE_LEN = 24; // XChaCha20-Poly1305 nonce
const EPH_PUB_LEN = 32; // X25519 public key
const REPLAY_WINDOW = 128;

// -- base64url helpers -------------------------------------------------------

/** Encode bytes to base64url without padding. */
export function base64urlEncode(bytes: Uint8Array): string {
  const b64 = btoa(String.fromCharCode(...bytes));
  return b64.replace(/\+/g, '-').replace(/\//g, '_').replace(/=/g, '');
}

/** Decode base64url (with or without padding). */
export function base64urlDecode(s: string): Uint8Array {
  const padded = s.replace(/-/g, '+').replace(/_/g, '/');
  const pad = (4 - (padded.length % 4)) % 4;
  const b64 = padded + '='.repeat(pad);
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

// -- keypair -----------------------------------------------------------------

export interface TunnelKeypair {
  publicKey: Uint8Array; // 32 bytes
  secretKey: Uint8Array; // 32 bytes
}

/** Generate a fresh X25519 keypair. */
export function generateKeypair(): TunnelKeypair {
  const secretKey = x25519.utils.randomSecretKey();
  const publicKey = x25519.getPublicKey(secretKey);
  cryptoLog('[crypto] keypair generated pubkey_len=%d', publicKey.length);
  return { publicKey, secretKey };
}

/** Derive a 32-byte X25519 shared secret. */
export function deriveSharedSecret(myPriv: Uint8Array, theirPub: Uint8Array): Uint8Array {
  const shared = x25519.getSharedSecret(myPriv, theirPub);
  cryptoLog('[crypto] shared secret derived');
  return shared;
}

// -- frame cipher ------------------------------------------------------------

/**
 * Seal `plaintext` into a versioned frame.
 * Output: version(1) || nonce(24) || ciphertext+tag
 */
export function seal(key: Uint8Array, plaintext: Uint8Array): Uint8Array {
  const nonce = randomBytes(NONCE_LEN);
  const cipher = xchacha20poly1305(key, nonce);
  const ciphertext = cipher.encrypt(plaintext);

  const frame = new Uint8Array(1 + NONCE_LEN + ciphertext.length);
  frame[0] = FRAME_VERSION;
  frame.set(nonce, 1);
  frame.set(ciphertext, 1 + NONCE_LEN);

  cryptoLog('[crypto] seal plaintext_len=%d frame_len=%d', plaintext.length, frame.length);
  return frame;
}

/**
 * Open a versioned frame.
 * Throws on version mismatch, replay, or authentication failure.
 */
export function open(key: Uint8Array, frame: Uint8Array, tracker: ReplayTracker): Uint8Array {
  if (frame.length === 0) {
    throw new Error('[crypto] empty frame');
  }
  if (frame[0] !== FRAME_VERSION) {
    throw new Error(`[crypto] unsupported frame version: 0x${frame[0].toString(16)}`);
  }
  if (frame.length < 1 + NONCE_LEN) {
    throw new Error('[crypto] frame too short for nonce');
  }

  const nonce = frame.slice(1, 1 + NONCE_LEN);
  const ciphertext = frame.slice(1 + NONCE_LEN);

  if (tracker.seen(nonce)) {
    throw new Error('[crypto] replayed nonce — frame rejected');
  }

  try {
    const cipher = xchacha20poly1305(key, nonce);
    const plaintext = cipher.decrypt(ciphertext);
    tracker.record(nonce);
    cryptoLog('[crypto] open frame_len=%d plaintext_len=%d', frame.length, plaintext.length);
    return plaintext;
  } catch (err) {
    cryptoErr('[crypto] authentication failed — tampered frame', err);
    throw new Error('[crypto] authentication failed — tampered frame');
  }
}

// -- sealed handshake --------------------------------------------------------

/**
 * Seal a handshake payload to the core's static public key using an ephemeral
 * X25519 keypair + XChaCha20-Poly1305.
 *
 * Output: 0x01 || eph_pub(32) || nonce(24) || ciphertext+tag
 *
 * Mirrors the wire format expected by bus.rs handle_tunnel_frame.
 */
export function sealHandshake(corePubkey: Uint8Array, payload: Uint8Array): Uint8Array {
  const eph = generateKeypair();
  const sharedKey = deriveSharedSecret(eph.secretKey, corePubkey);
  const nonce = randomBytes(NONCE_LEN);
  const cipher = xchacha20poly1305(sharedKey, nonce);
  const ciphertext = cipher.encrypt(payload);

  // 0x01 || eph_pub(32) || nonce(24) || ciphertext+tag
  const frame = new Uint8Array(1 + EPH_PUB_LEN + NONCE_LEN + ciphertext.length);
  frame[0] = FRAME_VERSION;
  frame.set(eph.publicKey, 1);
  frame.set(nonce, 1 + EPH_PUB_LEN);
  frame.set(ciphertext, 1 + EPH_PUB_LEN + NONCE_LEN);

  cryptoLog('[crypto] sealHandshake payload_len=%d frame_len=%d', payload.length, frame.length);
  return frame;
}

/**
 * Open a sealed handshake frame produced by `sealHandshake`.
 * Uses `myPriv` (core static key) to recover the plaintext.
 */
export function openHandshake(myPriv: Uint8Array, frame: Uint8Array): Uint8Array {
  if (frame.length < 1 + EPH_PUB_LEN + NONCE_LEN + 16) {
    throw new Error('[crypto] sealed-handshake frame too short');
  }
  if (frame[0] !== FRAME_VERSION) {
    throw new Error(`[crypto] bad handshake version: 0x${frame[0].toString(16)}`);
  }
  const ephPub = frame.slice(1, 1 + EPH_PUB_LEN);
  const nonce = frame.slice(1 + EPH_PUB_LEN, 1 + EPH_PUB_LEN + NONCE_LEN);
  const ciphertext = frame.slice(1 + EPH_PUB_LEN + NONCE_LEN);

  const sharedKey = deriveSharedSecret(myPriv, ephPub);
  try {
    const cipher = xchacha20poly1305(sharedKey, nonce);
    return cipher.decrypt(ciphertext);
  } catch {
    throw new Error('[crypto] handshake authentication failed');
  }
}

// -- replay tracker ----------------------------------------------------------

/** Sliding-window replay tracker over raw nonce bytes. */
export class ReplayTracker {
  private readonly window: Uint8Array[] = [];
  private readonly maxSize: number;

  constructor(windowSize = REPLAY_WINDOW) {
    this.maxSize = windowSize;
  }

  /** Returns true if `nonce` has been seen before. */
  seen(nonce: Uint8Array): boolean {
    return this.window.some(n => n.length === nonce.length && n.every((b, i) => b === nonce[i]));
  }

  /** Record a freshly-used nonce. Evicts oldest when window is full. */
  record(nonce: Uint8Array): void {
    if (this.window.length >= this.maxSize) {
      this.window.shift();
    }
    this.window.push(new Uint8Array(nonce));
  }
}
