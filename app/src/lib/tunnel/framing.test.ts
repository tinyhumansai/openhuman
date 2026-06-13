/**
 * Unit tests for tunnel/framing.ts
 */
import { describe, expect, it, vi } from 'vitest';

import { chunk, Envelope, Reassembler, TokenBucket } from './framing';

// -- chunk + reassemble round-trip -------------------------------------------

function makeEnvelope(payloadSize: number): Envelope {
  return { requestId: 'test-req-1', kind: 'response', seq: 0, payload: 'x'.repeat(payloadSize) };
}

describe('chunk', () => {
  it('returns a single frame for small payloads', () => {
    const env = makeEnvelope(100);
    const frames = chunk(env);
    expect(frames).toHaveLength(1);
  });

  it('splits large payloads into multiple chunks', () => {
    const env = makeEnvelope(200 * 1024); // 200 KB
    const frames = chunk(env);
    expect(frames.length).toBeGreaterThan(1);
  });

  it('produces multiple frames for large payloads (chunked)', () => {
    // Each output frame is a ChunkFrame JSON which has overhead; the test just
    // verifies that 200 KB produces multiple frames, each well under 100 KB.
    const env = makeEnvelope(200 * 1024);
    const frames = chunk(env);
    expect(frames.length).toBeGreaterThan(1);
    // Each frame carries at most 60 KB of raw data, plus base64 overhead (~33%)
    // plus JSON wrapper. 60 KB * 1.34 ≈ 80 KB; add wrapper ≈ 85 KB max.
    for (const f of frames) {
      expect(f.length).toBeLessThanOrEqual(90 * 1024);
    }
  });
});

describe('Reassembler', () => {
  it('passes through small (non-chunked) frames directly', () => {
    const r = new Reassembler();
    const env: Envelope = { requestId: 'r1', kind: 'request', seq: 0, payload: { method: 'ping' } };
    const raw = new TextEncoder().encode(JSON.stringify(env));
    const result = r.feed(raw);
    expect(result).not.toBeNull();
    expect(result!.requestId).toBe('r1');
  });

  it('chunk + reassemble round-trip for 200 KB payload', () => {
    const env = makeEnvelope(200 * 1024);
    const frames = chunk(env);
    const r = new Reassembler();

    let result: Envelope | null = null;
    for (let i = 0; i < frames.length - 1; i++) {
      const partial = r.feed(frames[i]);
      expect(partial).toBeNull(); // not yet complete
    }
    result = r.feed(frames[frames.length - 1]);

    expect(result).not.toBeNull();
    expect(result!.requestId).toBe('test-req-1');
    expect(result!.payload).toBe('x'.repeat(200 * 1024));
  });

  it('reassembles out-of-order chunks', () => {
    const env = makeEnvelope(200 * 1024);
    const frames = chunk(env);
    expect(frames.length).toBeGreaterThan(1);

    const r = new Reassembler();
    // Feed all but the first chunk in order, then feed first chunk last.
    const reordered = [...frames.slice(1), frames[0]];
    let result: Envelope | null = null;
    for (let i = 0; i < reordered.length; i++) {
      result = r.feed(reordered[i]);
    }
    expect(result).not.toBeNull();
    expect(result!.payload).toBe('x'.repeat(200 * 1024));
  });

  it('handles different requestIds concurrently', () => {
    const r = new Reassembler();
    const envA: Envelope = { requestId: 'A', kind: 'response', seq: 0, payload: 'aaa' };
    const envB: Envelope = { requestId: 'B', kind: 'response', seq: 0, payload: 'bbb' };

    const rawA = new TextEncoder().encode(JSON.stringify(envA));
    const rawB = new TextEncoder().encode(JSON.stringify(envB));

    const resultA = r.feed(rawA);
    const resultB = r.feed(rawB);

    expect(resultA!.requestId).toBe('A');
    expect(resultB!.requestId).toBe('B');
  });
});

// -- TokenBucket -------------------------------------------------------------

describe('TokenBucket', () => {
  it('allows up to burst capacity immediately', () => {
    const tb = new TokenBucket(100, 5);
    for (let i = 0; i < 5; i++) {
      expect(tb.tryConsume()).toBe(true);
    }
    expect(tb.tryConsume()).toBe(false); // burst exhausted
  });

  it('refills over time (using fake timers)', async () => {
    vi.useFakeTimers();
    const tb = new TokenBucket(100, 1); // 1 token burst
    expect(tb.tryConsume()).toBe(true);
    expect(tb.tryConsume()).toBe(false);

    // Advance 10ms (should add ~1 token at 100/s).
    await vi.advanceTimersByTimeAsync(10);
    expect(tb.tryConsume()).toBe(true);

    vi.useRealTimers();
  });

  it('consume() resolves after waiting for a token', async () => {
    vi.useFakeTimers();
    const tb = new TokenBucket(100, 1);
    tb.tryConsume(); // exhaust

    const done = vi.fn();
    const p = tb.consume().then(done);

    expect(done).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(15);
    await p;
    expect(done).toHaveBeenCalledOnce();

    vi.useRealTimers();
  });
});
