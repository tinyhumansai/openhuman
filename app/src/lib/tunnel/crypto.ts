/**
 * Tunnel crypto: X25519 key agreement + HKDF-SHA256 directional subkeys +
 * XChaCha20-Poly1305 frame encryption.
 *
 * Wire format (encrypted frame, v2):
 *   version(1=0x02) || nonce(24) || ciphertext+tag
 *
 * Frames produced with the previous `version=0x01` shape (single shared
 * key, same key in both directions, no KDF) are no longer accepted. Peers
 * see a distinctive "client upgrade required" error instead of a generic AEAD
 * authentication failure.
 *
 * Sealed-handshake format (device → core, first frame) uses marker 0x03,
 * an HKDF-derived key and authenticated header. The acknowledgement uses
 * marker 0x04 and a separate key.
 *
 * Mirrors crates/openhuman-core/src/security/devices/crypto.rs — keep in sync.
 */
import { xchacha20poly1305 } from '@noble/ciphers/chacha';
import { randomBytes } from '@noble/ciphers/webcrypto';
import { x25519 } from '@noble/curves/ed25519.js';
import { hkdf } from '@noble/hashes/hkdf.js';
import { sha256 } from '@noble/hashes/sha2.js';
import debug from 'debug';

const cryptoLog = debug('crypto');
const cryptoErr = debug('crypto:error');

// -- constants ---------------------------------------------------------------

/** Current frame version. v2 wraps HKDF-derived directional subkeys. */
export const FRAME_VERSION = 0x02;
/** Previous frame version. Surfaced so callers can recognise the legacy shape. */
export const LEGACY_FRAME_VERSION_V1 = 0x01;
export const HANDSHAKE_VERSION = 0x03;
export const HANDSHAKE_ACK_VERSION = 0x04;
const NONCE_LEN = 24; // XChaCha20-Poly1305 nonce
const EPH_PUB_LEN = 32; // X25519 public key
const REPLAY_WINDOW = 128;

/** HKDF info tags. Pinned strings — peer implementations MUST use the
 *  byte-identical values. */
export const HKDF_INFO_C2S = new TextEncoder().encode('openhuman-tunnel/v1/c2s');
export const HKDF_INFO_S2C = new TextEncoder().encode('openhuman-tunnel/v1/s2c');
const HKDF_INFO_HANDSHAKE = new TextEncoder().encode('openhuman-tunnel/v3/handshake');
const HKDF_INFO_HANDSHAKE_ACK = new TextEncoder().encode('openhuman-tunnel/v3/handshake-ack');
const HKDF_COMPAT_SALT = new TextEncoder().encode('openhuman-tunnel/compat');
const HKDF_INFO_COMPAT = new TextEncoder().encode('openhuman-tunnel/compat/symmetric');

function bootstrapKey(dh: Uint8Array, salt: Uint8Array, info: Uint8Array): Uint8Array {
  return hkdf(sha256, dh, salt, info, 32);
}

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

/** Reconstruct an X25519 keypair from a 32-byte private key. */
export function keypairFromSecretKey(secretKey: Uint8Array): TunnelKeypair {
  if (secretKey.length !== 32) {
    throw new Error(`[crypto] X25519 private key must be 32 bytes, got ${secretKey.length}`);
  }
  const publicKey = x25519.getPublicKey(secretKey);
  cryptoLog('[crypto] keypair restored pubkey_len=%d', publicKey.length);
  return { publicKey, secretKey };
}

/** Derive a 32-byte X25519 shared secret. */
export function deriveSharedSecret(myPriv: Uint8Array, theirPub: Uint8Array): Uint8Array {
  const shared = x25519.getSharedSecret(myPriv, theirPub);
  cryptoLog('[crypto] shared secret derived');
  return shared;
}

// -- session-key derivation (HKDF over static + ephemeral DH) ----------------

/**
 * Two 32-byte directional subkeys derived from the static + ephemeral DH
 * pair. `c2s` (client-to-server) is the key the iOS device uses to seal
 * outgoing frames; `s2c` is the inverse direction. A frame the server
 * emits cannot decrypt under the server's own opener — guards against
 * cross-direction reflection.
 */
interface SessionKeys {
  c2s: Uint8Array;
  s2c: Uint8Array;
}

/**
 * Derive `(c2s, s2c)` directional subkeys from the static + ephemeral DH
 * pair. Mirrors `derive_session_keys` in crates/openhuman-core/src/security/devices/crypto.rs:
 *
 *     ikm  = static_dh || eph_dh
 *     salt = client_eph_pub || server_eph_pub
 *     c2s  = HKDF-SHA256(ikm, salt, info = "openhuman-tunnel/v1/c2s", 32)
 *     s2c  = HKDF-SHA256(ikm, salt, info = "openhuman-tunnel/v1/s2c", 32)
 *
 * `static_dh` authenticates the peer via the long-term paired (QR-code)
 * keys; `eph_dh` is negotiated against fresh ephemeral keypairs at session
 * start to provide forward secrecy.
 */
export function deriveSessionKeys(
  staticDh: Uint8Array,
  ephDh: Uint8Array,
  clientEphPub: Uint8Array,
  serverEphPub: Uint8Array
): SessionKeys {
  if (staticDh.length !== 32 || ephDh.length !== 32) {
    throw new Error('[crypto] DH inputs must be 32 bytes each');
  }
  if (clientEphPub.length !== 32 || serverEphPub.length !== 32) {
    throw new Error('[crypto] ephemeral public keys must be 32 bytes each');
  }
  const ikm = new Uint8Array(64);
  ikm.set(staticDh, 0);
  ikm.set(ephDh, 32);

  const salt = new Uint8Array(64);
  salt.set(clientEphPub, 0);
  salt.set(serverEphPub, 32);

  const c2s = hkdf(sha256, ikm, salt, HKDF_INFO_C2S, 32);
  const s2c = hkdf(sha256, ikm, salt, HKDF_INFO_S2C, 32);

  cryptoLog('[crypto] derived directional session keys (HKDF-SHA256)');
  return { c2s, s2c };
}

/** Which side of the tunnel is operating the cipher. */
type TunnelRole = 'client' | 'server';

/**
 * Stateful directional cipher mirroring Rust `TunnelCipher::for_role`. Holds
 * two XChaCha20-Poly1305 instances: `sealKey` for the local direction and
 * `openKey` for the peer's.
 */
export class TunnelCipher {
  private readonly sealKey: Uint8Array;
  private readonly openKey: Uint8Array;
  private readonly replay: ReplayTracker;

  constructor(role: TunnelRole, keys: SessionKeys, replay: ReplayTracker = new ReplayTracker()) {
    if (role === 'client') {
      this.sealKey = keys.c2s;
      this.openKey = keys.s2c;
    } else {
      this.sealKey = keys.s2c;
      this.openKey = keys.c2s;
    }
    this.replay = replay;
    cryptoLog('[crypto] TunnelCipher created role=%s (directional subkeys)', role);
  }

  /**
   * Seal `plaintext` into a versioned frame.
   * Output: version(1=0x02) || nonce(24) || ciphertext+tag.
   */
  seal(plaintext: Uint8Array): Uint8Array {
    return sealWithKey(this.sealKey, plaintext);
  }

  /**
   * Open a versioned frame. Throws on version mismatch (including legacy
   * v1 frames), replay, or authentication failure.
   */
  open(frame: Uint8Array): Uint8Array {
    return openWithKey(this.openKey, frame, this.replay);
  }
}

// -- frame cipher ------------------------------------------------------------

function sealWithKey(key: Uint8Array, plaintext: Uint8Array): Uint8Array {
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

function openWithKey(key: Uint8Array, frame: Uint8Array, tracker: ReplayTracker): Uint8Array {
  if (frame.length === 0) {
    throw new Error('[crypto] empty frame');
  }
  if (frame[0] === LEGACY_FRAME_VERSION_V1) {
    throw new Error(
      '[crypto] UnsupportedFrameVersion: legacy v1 frame rejected — peer must upgrade to v2 directional subkeys before reconnecting'
    );
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

/**
 * Compatibility single-key helpers for local fixtures. Production session
 * frames go through `TunnelCipher` with directional subkeys.
 */
export function seal(key: Uint8Array, plaintext: Uint8Array): Uint8Array {
  return sealWithKey(bootstrapKey(key, HKDF_COMPAT_SALT, HKDF_INFO_COMPAT), plaintext);
}

/**
 * Legacy single-key open. See {@link seal} for the rationale.
 */
export function open(key: Uint8Array, frame: Uint8Array, tracker: ReplayTracker): Uint8Array {
  return openWithKey(bootstrapKey(key, HKDF_COMPAT_SALT, HKDF_INFO_COMPAT), frame, tracker);
}

// -- sealed handshake --------------------------------------------------------

/**
 * Seal a handshake payload to the core's static public key using an ephemeral
 * X25519 keypair + XChaCha20-Poly1305.
 *
 * Output: 0x03 || eph_pub(32) || nonce(24) || ciphertext+tag
 *
 * Mirrors the wire format expected by bus.rs handle_tunnel_frame.
 */
export function sealHandshake(corePubkey: Uint8Array, payload: Uint8Array): Uint8Array {
  if (corePubkey.length !== 32) {
    throw new Error('[crypto] core public key must be 32 bytes');
  }
  const eph = generateKeypair();
  const dh = deriveSharedSecret(eph.secretKey, corePubkey);
  const salt = new Uint8Array(64);
  salt.set(eph.publicKey);
  salt.set(corePubkey, 32);
  const key = bootstrapKey(dh, salt, HKDF_INFO_HANDSHAKE);
  const nonce = randomBytes(NONCE_LEN);
  const header = new Uint8Array(1 + EPH_PUB_LEN);
  header[0] = HANDSHAKE_VERSION;
  header.set(eph.publicKey, 1);
  const cipher = xchacha20poly1305(key, nonce, header);
  const ciphertext = cipher.encrypt(payload);

  // 0x03 || eph_pub(32) || nonce(24) || ciphertext+tag
  const frame = new Uint8Array(1 + EPH_PUB_LEN + NONCE_LEN + ciphertext.length);
  frame[0] = HANDSHAKE_VERSION;
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
  if (frame[0] !== HANDSHAKE_VERSION) {
    throw new Error(`[crypto] bad handshake version: 0x${frame[0].toString(16)}`);
  }
  const ephPub = frame.slice(1, 1 + EPH_PUB_LEN);
  const nonce = frame.slice(1 + EPH_PUB_LEN, 1 + EPH_PUB_LEN + NONCE_LEN);
  const ciphertext = frame.slice(1 + EPH_PUB_LEN + NONCE_LEN);

  const dh = deriveSharedSecret(myPriv, ephPub);
  const salt = new Uint8Array(64);
  salt.set(ephPub);
  salt.set(x25519.getPublicKey(myPriv), 32);
  const key = bootstrapKey(dh, salt, HKDF_INFO_HANDSHAKE);
  try {
    const cipher = xchacha20poly1305(key, nonce, frame.slice(0, 1 + EPH_PUB_LEN));
    return cipher.decrypt(ciphertext);
  } catch {
    throw new Error('[crypto] handshake authentication failed');
  }
}

/** Open the core's bootstrap acknowledgement using a key distinct from both
 * the device handshake key and the directional session keys. */
export function openHandshakeAck(
  staticDh: Uint8Array,
  clientEphPub: Uint8Array,
  frame: Uint8Array
): Uint8Array {
  if (staticDh.length !== 32 || clientEphPub.length !== 32) {
    throw new Error('[crypto] acknowledgement key inputs must be 32 bytes');
  }
  if (frame.length < 1 + NONCE_LEN + 16 || frame[0] !== HANDSHAKE_ACK_VERSION) {
    throw new Error('[crypto] unsupported handshake acknowledgement');
  }
  const aad = new Uint8Array(33);
  aad[0] = HANDSHAKE_ACK_VERSION;
  aad.set(clientEphPub, 1);
  const key = bootstrapKey(staticDh, clientEphPub, HKDF_INFO_HANDSHAKE_ACK);
  try {
    return xchacha20poly1305(key, frame.slice(1, 1 + NONCE_LEN), aad).decrypt(
      frame.slice(1 + NONCE_LEN)
    );
  } catch {
    throw new Error('[crypto] handshake acknowledgement authentication failed');
  }
}

/** Encode an acknowledgement for a core-side peer or local protocol fixture. */
export function sealHandshakeAck(
  staticDh: Uint8Array,
  clientEphPub: Uint8Array,
  plaintext: Uint8Array
): Uint8Array {
  if (staticDh.length !== 32 || clientEphPub.length !== 32) {
    throw new Error('[crypto] acknowledgement key inputs must be 32 bytes');
  }
  const aad = new Uint8Array(33);
  aad[0] = HANDSHAKE_ACK_VERSION;
  aad.set(clientEphPub, 1);
  const key = bootstrapKey(staticDh, clientEphPub, HKDF_INFO_HANDSHAKE_ACK);
  const nonce = randomBytes(NONCE_LEN);
  const ciphertext = xchacha20poly1305(key, nonce, aad).encrypt(plaintext);
  const frame = new Uint8Array(1 + NONCE_LEN + ciphertext.length);
  frame[0] = HANDSHAKE_ACK_VERSION;
  frame.set(nonce, 1);
  frame.set(ciphertext, 1 + NONCE_LEN);
  return frame;
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
