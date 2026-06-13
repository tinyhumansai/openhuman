/**
 * Unit tests for tunnel/crypto.ts
 */
import { describe, expect, it } from 'vitest';

import {
  base64urlDecode,
  base64urlEncode,
  deriveSessionKeys,
  deriveSharedSecret,
  FRAME_VERSION,
  generateKeypair,
  HKDF_INFO_C2S,
  HKDF_INFO_S2C,
  keypairFromSecretKey,
  LEGACY_FRAME_VERSION_V1,
  open,
  openHandshake,
  ReplayTracker,
  seal,
  sealHandshake,
  TunnelCipher,
} from './crypto';

// -- base64url helpers -------------------------------------------------------

describe('base64url helpers', () => {
  it('round-trips arbitrary bytes', () => {
    const bytes = new Uint8Array([0, 1, 2, 255, 128, 64]);
    expect(base64urlDecode(base64urlEncode(bytes))).toEqual(bytes);
  });

  it('produces no padding characters', () => {
    const s = base64urlEncode(new Uint8Array(10));
    expect(s).not.toMatch(/=/);
  });

  it('uses - and _ instead of + and /', () => {
    // Generate bytes that would produce + and / in standard base64.
    // 0xFB = 11111011 → standard base64 uses '+' and '/'.
    for (let i = 0; i < 100; i++) {
      const b = new Uint8Array([0xfb, 0xff, 0xfe]);
      const s = base64urlEncode(b);
      expect(s).not.toMatch(/\+|\/|=/);
    }
  });
});

// -- keypair generation and DH -----------------------------------------------

describe('generateKeypair', () => {
  it('returns 32-byte keys', () => {
    const kp = generateKeypair();
    expect(kp.publicKey).toHaveLength(32);
    expect(kp.secretKey).toHaveLength(32);
  });

  it('two keypairs are different', () => {
    const a = generateKeypair();
    const b = generateKeypair();
    expect(a.publicKey).not.toEqual(b.publicKey);
  });
});

describe('keypairFromSecretKey', () => {
  it('restores the public key from a private key', () => {
    const original = generateKeypair();
    const restored = keypairFromSecretKey(original.secretKey);
    expect(restored.secretKey).toEqual(original.secretKey);
    expect(restored.publicKey).toEqual(original.publicKey);
  });

  it('rejects non-X25519 private key lengths', () => {
    expect(() => keypairFromSecretKey(new Uint8Array(31))).toThrow(/32 bytes/i);
  });
});

describe('deriveSharedSecret', () => {
  it('both sides derive the same secret', () => {
    const alice = generateKeypair();
    const bob = generateKeypair();
    const aliceShared = deriveSharedSecret(alice.secretKey, bob.publicKey);
    const bobShared = deriveSharedSecret(bob.secretKey, alice.publicKey);
    expect(aliceShared).toEqual(bobShared);
  });
});

// -- seal / open round-trip --------------------------------------------------

describe('seal / open', () => {
  function makeKey(): Uint8Array {
    const a = generateKeypair();
    const b = generateKeypair();
    return deriveSharedSecret(a.secretKey, b.publicKey);
  }

  it('round-trip encrypts and decrypts', () => {
    const key = makeKey();
    const tracker = new ReplayTracker();
    const plaintext = new TextEncoder().encode('hello tunnel');
    const frame = seal(key, plaintext);
    const recovered = open(key, frame, tracker);
    expect(Array.from(recovered)).toEqual(Array.from(plaintext));
  });

  it('rejects tampered frame', () => {
    const key = makeKey();
    const tracker = new ReplayTracker();
    const frame = seal(key, new TextEncoder().encode('data'));
    frame[frame.length - 1] ^= 0xff; // flip last byte
    expect(() => open(key, frame, tracker)).toThrow(/tampered|authentication/i);
  });

  it('rejects replayed nonce', () => {
    const key = makeKey();
    const tracker = new ReplayTracker();
    const frame = seal(key, new TextEncoder().encode('replay me'));
    open(key, frame, tracker); // first: ok
    expect(() => open(key, frame, tracker)).toThrow(/replayed nonce/i);
  });

  it('rejects wrong version byte', () => {
    const key = makeKey();
    const tracker = new ReplayTracker();
    const frame = seal(key, new TextEncoder().encode('version test'));
    const badFrame = new Uint8Array(frame);
    badFrame[0] = 0x99;
    expect(() => open(key, badFrame, tracker)).toThrow(/unsupported frame version/i);
  });

  it('rejects empty frame', () => {
    const key = makeKey();
    const tracker = new ReplayTracker();
    expect(() => open(key, new Uint8Array(0), tracker)).toThrow(/empty frame/i);
  });
});

// -- sealed handshake --------------------------------------------------------

describe('sealHandshake / openHandshake', () => {
  it('round-trip via sealHandshake + openHandshake', () => {
    const core = generateKeypair();
    const payload = new TextEncoder().encode('device_pubkey_b64url');
    const frame = sealHandshake(core.publicKey, payload);
    const recovered = openHandshake(core.secretKey, frame);
    expect(Array.from(recovered)).toEqual(Array.from(payload));
  });

  it('frame starts with version byte 0x01', () => {
    const core = generateKeypair();
    const frame = sealHandshake(core.publicKey, new Uint8Array(16));
    expect(frame[0]).toBe(0x01);
  });

  it('rejects tampered handshake frame', () => {
    const core = generateKeypair();
    const frame = sealHandshake(core.publicKey, new TextEncoder().encode('payload'));
    const bad = new Uint8Array(frame);
    bad[bad.length - 1] ^= 0xff;
    expect(() => openHandshake(core.secretKey, bad)).toThrow(/authentication failed/i);
  });

  it('rejects frame that is too short', () => {
    const core = generateKeypair();
    const tinyFrame = new Uint8Array([0x01, 0x00, 0x01]);
    expect(() => openHandshake(core.secretKey, tinyFrame)).toThrow(/too short/i);
  });
});

// -- HKDF directional subkeys + frame v2 -------------------------------------

describe('deriveSessionKeys', () => {
  const staticDh = new Uint8Array(32).fill(0x11);
  const ephDh = new Uint8Array(32).fill(0x22);
  const clientEph = new Uint8Array(32).fill(0x33);
  const serverEph = new Uint8Array(32).fill(0x44);

  it('derives distinct directional subkeys for the same IKM+salt', () => {
    const keys = deriveSessionKeys(staticDh, ephDh, clientEph, serverEph);
    expect(keys.c2s).toHaveLength(32);
    expect(keys.s2c).toHaveLength(32);
    expect(Array.from(keys.c2s)).not.toEqual(Array.from(keys.s2c));
  });

  it('is deterministic across re-derivation', () => {
    const a = deriveSessionKeys(staticDh, ephDh, clientEph, serverEph);
    const b = deriveSessionKeys(staticDh, ephDh, clientEph, serverEph);
    expect(Array.from(a.c2s)).toEqual(Array.from(b.c2s));
    expect(Array.from(a.s2c)).toEqual(Array.from(b.s2c));
  });

  it('pins the HKDF info tags so a rename fails loudly', () => {
    // The exact byte strings are part of the wire contract with the Rust
    // peer; if these change, peers fail to derive the same session keys
    // even though the underlying secret is identical.
    expect(new TextDecoder().decode(HKDF_INFO_C2S)).toBe('openhuman-tunnel/v1/c2s');
    expect(new TextDecoder().decode(HKDF_INFO_S2C)).toBe('openhuman-tunnel/v1/s2c');
  });

  it('rejects DH inputs that are not 32 bytes', () => {
    expect(() => deriveSessionKeys(new Uint8Array(31), ephDh, clientEph, serverEph)).toThrow(
      /32 bytes/i
    );
    expect(() => deriveSessionKeys(staticDh, ephDh, new Uint8Array(0), serverEph)).toThrow(
      /32 bytes/i
    );
  });

  it('static-only adversary cannot recover prior session keys (FS sanity)', () => {
    const a = deriveSessionKeys(staticDh, new Uint8Array(32).fill(0xaa), clientEph, serverEph);
    const b = deriveSessionKeys(staticDh, new Uint8Array(32).fill(0xbb), clientEph, serverEph);
    expect(Array.from(a.c2s)).not.toEqual(Array.from(b.c2s));
    expect(Array.from(a.s2c)).not.toEqual(Array.from(b.s2c));
  });
});

describe('TunnelCipher (directional v2)', () => {
  const keys = deriveSessionKeys(
    new Uint8Array(32).fill(0x55),
    new Uint8Array(32).fill(0x66),
    new Uint8Array(32).fill(0x77),
    new Uint8Array(32).fill(0x88)
  );

  it('client→server round-trips', () => {
    const client = new TunnelCipher('client', keys);
    const server = new TunnelCipher('server', keys);
    const plaintext = new TextEncoder().encode('hi from client');
    const frame = client.seal(plaintext);
    expect(frame[0]).toBe(FRAME_VERSION);
    const recovered = server.open(frame);
    expect(Array.from(recovered)).toEqual(Array.from(plaintext));
  });

  it('server→client round-trips', () => {
    const client = new TunnelCipher('client', keys);
    const server = new TunnelCipher('server', keys);
    const plaintext = new TextEncoder().encode('hi from server');
    const frame = server.seal(plaintext);
    const recovered = client.open(frame);
    expect(Array.from(recovered)).toEqual(Array.from(plaintext));
  });

  it('cross-direction reflection fails (server cannot decrypt its own outbound frame)', () => {
    const server = new TunnelCipher('server', keys);
    const serverOpener = new TunnelCipher('server', keys);
    const frame = server.seal(new TextEncoder().encode('frame from server'));
    expect(() => serverOpener.open(frame)).toThrow(/authentication failed/i);
  });

  it('legacy v1 frames are explicitly rejected with re-pair hint', () => {
    // Hand-roll a v1-shaped frame: 0x01 || nonce(24) || ct(16 bytes).
    const v1Frame = new Uint8Array(1 + 24 + 16);
    v1Frame[0] = LEGACY_FRAME_VERSION_V1;
    const client = new TunnelCipher('client', keys);
    expect(() => client.open(v1Frame)).toThrow(/UnsupportedFrameVersion/);
    expect(() => client.open(v1Frame)).toThrow(/re-pair/);
  });
});

// -- ReplayTracker -----------------------------------------------------------

describe('ReplayTracker', () => {
  it('accepts fresh nonces', () => {
    const tracker = new ReplayTracker(4);
    const nonce = new Uint8Array([1, 2, 3]);
    expect(tracker.seen(nonce)).toBe(false);
    tracker.record(nonce);
    expect(tracker.seen(nonce)).toBe(true);
  });

  it('evicts oldest nonce when window is full', () => {
    const tracker = new ReplayTracker(2);
    const n1 = new Uint8Array([1]);
    const n2 = new Uint8Array([2]);
    const n3 = new Uint8Array([3]);
    tracker.record(n1);
    tracker.record(n2);
    tracker.record(n3); // evicts n1
    expect(tracker.seen(n1)).toBe(false); // evicted
    expect(tracker.seen(n2)).toBe(true);
    expect(tracker.seen(n3)).toBe(true);
  });
});
