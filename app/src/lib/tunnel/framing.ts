/**
 * Tunnel framing: request/response/streaming envelopes over encrypted frames.
 *
 * Envelope JSON schema:
 *   { requestId, kind, seq, payload }
 *
 * Large envelopes are split into ≤60 KB chunks, each wrapped in:
 *   { requestId, kind: "chunk", seq, total, data: base64 }
 *
 * Rate limiting: TokenBucket at 100 frames/s with burst.
 */
import debug from 'debug';

const framingLog = debug('framing');

// -- constants ---------------------------------------------------------------

const CHUNK_SIZE = 60 * 1024; // 60 KB max per chunk (headroom under 64 KB)

// -- types -------------------------------------------------------------------

export type EnvelopeKind = 'request' | 'response' | 'stream-chunk' | 'stream-end' | 'error';

export interface Envelope {
  requestId: string;
  kind: EnvelopeKind;
  seq: number;
  payload: unknown;
}

interface ChunkFrame {
  requestId: string;
  kind: 'chunk';
  seq: number; // chunk index (0-based)
  total: number; // total chunks
  data: string; // base64-encoded fragment
}

// -- chunking ----------------------------------------------------------------

/**
 * Encode an envelope to UTF-8 and split into ≤60 KB chunks.
 * Returns a single encoded Uint8Array when the envelope fits in one frame.
 */
export function chunk(envelope: Envelope): Uint8Array[] {
  const json = JSON.stringify(envelope);
  const encoded = new TextEncoder().encode(json);

  if (encoded.length <= CHUNK_SIZE) {
    return [encoded];
  }

  // Split into chunks.
  const total = Math.ceil(encoded.length / CHUNK_SIZE);
  const chunks: Uint8Array[] = [];

  for (let i = 0; i < total; i++) {
    const slice = encoded.slice(i * CHUNK_SIZE, (i + 1) * CHUNK_SIZE);
    // base64 encode the raw bytes of this slice
    const data = btoa(String.fromCharCode(...slice));
    const frame: ChunkFrame = { requestId: envelope.requestId, kind: 'chunk', seq: i, total, data };
    chunks.push(new TextEncoder().encode(JSON.stringify(frame)));
  }

  framingLog('[framing] chunk requestId=%s total=%d', envelope.requestId, total);
  return chunks;
}

// -- reassembler -------------------------------------------------------------

interface PendingAssembly {
  total: number;
  parts: Map<number, Uint8Array>; // seq -> raw bytes of the slice
}

/** Collects chunk frames by requestId and emits complete Envelopes. */
export class Reassembler {
  private readonly pending = new Map<string, PendingAssembly>();

  /**
   * Feed a raw frame (UTF-8 bytes) into the reassembler.
   *
   * - If the frame is a complete envelope, parse and return it immediately.
   * - If the frame is a chunk, buffer it and return the assembled envelope
   *   once all chunks have arrived.
   * - Returns null if assembly is incomplete.
   */
  feed(raw: Uint8Array): Envelope | null {
    let parsed: unknown;
    try {
      parsed = JSON.parse(new TextDecoder().decode(raw));
    } catch {
      framingLog('[framing] Reassembler: failed to parse frame');
      return null;
    }

    const obj = parsed as Record<string, unknown>;

    if (obj.kind === 'chunk') {
      return this.handleChunk(obj as unknown as ChunkFrame);
    }

    // Complete envelope.
    return parsed as Envelope;
  }

  private handleChunk(frame: ChunkFrame): Envelope | null {
    const { requestId, seq, total, data } = frame;

    if (!this.pending.has(requestId)) {
      this.pending.set(requestId, { total, parts: new Map() });
    }

    const entry = this.pending.get(requestId)!;

    // Decode base64 fragment back to bytes.
    const binary = atob(data);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) {
      bytes[i] = binary.charCodeAt(i);
    }
    entry.parts.set(seq, bytes);

    framingLog('[framing] chunk seq=%d/%d requestId=%s', seq + 1, total, requestId);

    if (entry.parts.size < total) {
      return null; // still waiting
    }

    // All chunks present — reassemble in order.
    const ordered = Array.from({ length: total }, (_, i) => entry.parts.get(i)!);
    const totalLen = ordered.reduce((acc, b) => acc + b.length, 0);
    const combined = new Uint8Array(totalLen);
    let offset = 0;
    for (const part of ordered) {
      combined.set(part, offset);
      offset += part.length;
    }

    this.pending.delete(requestId);

    try {
      const env = JSON.parse(new TextDecoder().decode(combined)) as Envelope;
      framingLog('[framing] reassembled requestId=%s', requestId);
      return env;
    } catch {
      framingLog('[framing] reassemble parse failed requestId=%s', requestId);
      return null;
    }
  }
}

// -- token bucket rate limiter -----------------------------------------------

/**
 * Token bucket rate limiter.
 * Default: 100 frames/s with burst capacity of 100.
 */
export class TokenBucket {
  private tokens: number;
  private readonly capacity: number;
  private readonly refillRate: number; // tokens per ms
  private lastRefill: number;

  constructor(ratePerSecond = 100, burstCapacity = 100) {
    this.capacity = burstCapacity;
    this.tokens = burstCapacity;
    this.refillRate = ratePerSecond / 1000;
    this.lastRefill = Date.now();
  }

  /**
   * Attempt to consume one token.
   * Returns true if allowed, false if rate-limited.
   */
  tryConsume(): boolean {
    this.refill();
    if (this.tokens >= 1) {
      this.tokens -= 1;
      return true;
    }
    return false;
  }

  /**
   * Wait until a token is available, then consume it.
   * Resolves after the appropriate delay.
   */
  async consume(): Promise<void> {
    this.refill();
    if (this.tokens >= 1) {
      this.tokens -= 1;
      return;
    }
    // How long until we have one token?
    const waitMs = Math.ceil((1 - this.tokens) / this.refillRate);
    await new Promise<void>(resolve => setTimeout(resolve, waitMs));
    this.refill();
    this.tokens = Math.max(0, this.tokens - 1);
  }

  private refill(): void {
    const now = Date.now();
    const elapsed = now - this.lastRefill;
    this.tokens = Math.min(this.capacity, this.tokens + elapsed * this.refillRate);
    this.lastRefill = now;
  }
}
