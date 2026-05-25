/**
 * Unit tests for tunnel/crypto.ts
 */
import { describe, expect, it } from 'vitest';

import {
  base64urlDecode,
  base64urlEncode,
  deriveSharedSecret,
  generateKeypair,
  open,
  openHandshake,
  ReplayTracker,
  seal,
  sealHandshake,
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
