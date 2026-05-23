/**
 * TunnelTransport — socket.io client using the backend tunnel relay.
 *
 * Handles:
 * - Connecting to the backend with `tunnel:connect` (role: "client")
 * - Sending RPC calls as `tunnel:frame` events (E2E encrypted + chunked)
 * - Receiving response frames, decrypting, and resolving the matching request
 * - First frame: sealed handshake (sends device pubkey encrypted to core pubkey)
 * - Subsequent frames: symmetric XChaCha20-Poly1305 encryption
 *
 * Key material is never logged. Only lengths and first-4-char prefixes appear.
 */
import debug from 'debug';
import { io, Socket } from 'socket.io-client';

import {
  base64urlDecode,
  base64urlEncode,
  deriveSharedSecret,
  generateKeypair,
  open,
  ReplayTracker,
  seal,
  sealHandshake,
  type TunnelKeypair,
} from '../../lib/tunnel/crypto';
import { chunk, Envelope, Reassembler, TokenBucket } from '../../lib/tunnel/framing';
import type { CoreTransport } from './CoreTransport';

const log = debug('transport:tunnel');
const logErr = debug('transport:tunnel:error');

// -- types -------------------------------------------------------------------

interface PendingCall {
  resolve: (value: unknown) => void;
  reject: (err: Error) => void;
  timeoutId: ReturnType<typeof setTimeout>;
}

interface StreamChunkHandler {
  push: (value: unknown) => void;
  finish: () => void;
  error: (err: Error) => void;
}

// -- TunnelTransport ---------------------------------------------------------

export class TunnelTransport implements CoreTransport {
  readonly kind = 'tunnel' as const;

  private socket: Socket | null = null;
  private sessionKey: Uint8Array | null = null; // derived after handshake
  private deviceKeypair: TunnelKeypair | null = null;
  private readonly replayTracker = new ReplayTracker();
  private readonly reassembler = new Reassembler();
  private readonly rateLimiter = new TokenBucket(100, 100);

  private readonly pending = new Map<string, PendingCall>();
  private readonly streams = new Map<string, StreamChunkHandler>();

  private _connectPromise: Promise<void> | null = null;

  constructor(
    private readonly backendUrl: string,
    private readonly channelId: string,
    private readonly corePubkeyB64: string,
    private readonly authToken: string, // sessionToken (reconnect) or pairingToken (first)
    private readonly role: 'client' = 'client',
    private readonly callTimeoutMs: number = 30_000
  ) {
    // Generate device keypair on construction.
    this.deviceKeypair = generateKeypair();
    log('[tunnel] created channelId=%s corePubkey=%s…', channelId, corePubkeyB64.slice(0, 4));
  }

  // -- connect ---------------------------------------------------------------

  private ensureConnected(): Promise<void> {
    if (this._connectPromise) return this._connectPromise;

    this._connectPromise = new Promise<void>((resolve, reject) => {
      log('[tunnel] connecting to %s channelId=%s', this.backendUrl, this.channelId);

      const socket = io(this.backendUrl, {
        transports: ['websocket', 'polling'],
        reconnection: true,
        reconnectionDelay: 1000,
        reconnectionAttempts: 10,
        forceNew: true,
      });

      this.socket = socket;

      socket.on('connect', () => {
        log('[tunnel] socket connected, emitting tunnel:connect channelId=%s', this.channelId);
        socket.emit('tunnel:connect', {
          channelId: this.channelId,
          role: this.role,
          token: this.authToken,
        });
      });

      socket.on('tunnel:connected', () => {
        log('[tunnel] tunnel:connected ack received, performing handshake');
        // Send sealed handshake frame.
        void this.sendHandshake().then(resolve).catch(reject);
      });

      socket.on('tunnel:frame', (data: unknown) => {
        void this.handleIncomingFrame(data);
      });

      socket.on('tunnel:error', (err: unknown) => {
        logErr('[tunnel] tunnel:error %o', err);
        const errMsg = typeof err === 'string' ? err : JSON.stringify(err);
        reject(new Error(`[tunnel] server error: ${errMsg}`));
        this.rejectAllPending(new Error(`[tunnel] server error: ${errMsg}`));
      });

      socket.on('disconnect', (reason: string) => {
        log('[tunnel] disconnected reason=%s', reason);
        this.sessionKey = null;
        this._connectPromise = null;
      });

      socket.on('connect_error', (err: Error) => {
        logErr('[tunnel] connect_error %s', err.message);
        reject(err);
        this._connectPromise = null;
      });
    });

    return this._connectPromise;
  }

  // -- handshake -------------------------------------------------------------

  private async sendHandshake(): Promise<void> {
    if (!this.deviceKeypair) throw new Error('[tunnel] no device keypair');

    const corePubkey = base64urlDecode(this.corePubkeyB64);
    const devicePubkeyB64 = base64urlEncode(this.deviceKeypair.publicKey);

    // Device pubkey payload (base64url-encoded, UTF-8).
    const payload = new TextEncoder().encode(devicePubkeyB64);

    // Seal the handshake payload to the core's public key.
    const handshakeFrame = sealHandshake(corePubkey, payload);
    const frameB64 = base64urlEncode(handshakeFrame);

    log('[tunnel] sending sealed handshake frame_len=%d', handshakeFrame.length);
    this.socket!.emit('tunnel:frame', { channelId: this.channelId, payload: frameB64 });

    // Derive session key from static keys (both sides derive the same key).
    this.sessionKey = deriveSharedSecret(this.deviceKeypair.secretKey, corePubkey);

    log('[tunnel] handshake complete, session key derived');
  }

  // -- incoming frames -------------------------------------------------------

  private async handleIncomingFrame(data: unknown): Promise<void> {
    const obj = data as Record<string, unknown>;
    const payloadB64 = typeof obj?.payload === 'string' ? obj.payload : null;
    if (!payloadB64) {
      logErr('[tunnel] incoming frame missing payload');
      return;
    }

    if (!this.sessionKey) {
      log('[tunnel] frame received before session key — ignoring');
      return;
    }

    let frameBytes: Uint8Array;
    try {
      frameBytes = base64urlDecode(payloadB64);
    } catch (err) {
      logErr('[tunnel] bad base64url in incoming frame: %s', (err as Error).message);
      return;
    }

    let plaintext: Uint8Array;
    try {
      plaintext = open(this.sessionKey, frameBytes, this.replayTracker);
    } catch (err) {
      logErr('[tunnel] frame decryption failed: %s', (err as Error).message);
      return;
    }

    const envelope = this.reassembler.feed(plaintext);
    if (!envelope) return; // waiting for more chunks

    this.dispatchEnvelope(envelope);
  }

  private dispatchEnvelope(envelope: Envelope): void {
    const { requestId, kind } = envelope;

    if (kind === 'stream-chunk' || kind === 'stream-end') {
      const handler = this.streams.get(requestId);
      if (!handler) return;
      if (kind === 'stream-chunk') {
        handler.push(envelope.payload);
      } else {
        handler.finish();
        this.streams.delete(requestId);
      }
      return;
    }

    if (kind === 'error') {
      const pending = this.pending.get(requestId);
      if (pending) {
        clearTimeout(pending.timeoutId);
        this.pending.delete(requestId);
        pending.reject(new Error(String(envelope.payload ?? 'tunnel error')));
      }
      const stream = this.streams.get(requestId);
      if (stream) {
        stream.error(new Error(String(envelope.payload ?? 'tunnel error')));
        this.streams.delete(requestId);
      }
      return;
    }

    if (kind === 'response') {
      const pending = this.pending.get(requestId);
      if (!pending) return;
      clearTimeout(pending.timeoutId);
      this.pending.delete(requestId);
      pending.resolve(envelope.payload);
      return;
    }
  }

  // -- send ------------------------------------------------------------------

  private async sendEnvelope(envelope: Envelope): Promise<void> {
    if (!this.sessionKey) throw new Error('[tunnel] no session key — handshake incomplete');

    await this.rateLimiter.consume();

    const chunks = chunk(envelope);
    for (const raw of chunks) {
      const encrypted = seal(this.sessionKey, raw);
      const frameB64 = base64urlEncode(encrypted);
      this.socket!.emit('tunnel:frame', { channelId: this.channelId, payload: frameB64 });
    }

    log(
      '[tunnel] sent %s requestId=%s chunks=%d',
      envelope.kind,
      envelope.requestId,
      chunks.length
    );
  }

  // -- CoreTransport ---------------------------------------------------------

  async call<T>(method: string, params: unknown, opts?: { signal?: AbortSignal }): Promise<T> {
    await this.ensureConnected();

    const requestId = crypto.randomUUID();
    const envelope: Envelope = { requestId, kind: 'request', seq: 0, payload: { method, params } };

    return new Promise<T>((resolve, reject) => {
      const timeoutId = setTimeout(() => {
        this.pending.delete(requestId);
        reject(new Error(`[tunnel] ${method} timed out after ${this.callTimeoutMs}ms`));
      }, this.callTimeoutMs);

      opts?.signal?.addEventListener('abort', () => {
        clearTimeout(timeoutId);
        this.pending.delete(requestId);
        reject(new Error(`[tunnel] ${method} aborted`));
      });

      this.pending.set(requestId, { resolve: v => resolve(v as T), reject, timeoutId });

      void this.sendEnvelope(envelope).catch((err: Error) => {
        clearTimeout(timeoutId);
        this.pending.delete(requestId);
        reject(err);
      });
    });
  }

  async *stream<T>(
    method: string,
    params: unknown,
    opts?: { signal?: AbortSignal }
  ): AsyncIterable<T> {
    await this.ensureConnected();

    const requestId = crypto.randomUUID();
    const envelope: Envelope = {
      requestId,
      kind: 'request',
      seq: 0,
      payload: { method, params, stream: true },
    };

    const queue: T[] = [];
    let finished = false;
    let streamError: Error | null = null;
    let notify: (() => void) | null = null;

    this.streams.set(requestId, {
      push: v => {
        queue.push(v as T);
        notify?.();
      },
      finish: () => {
        finished = true;
        notify?.();
      },
      error: err => {
        streamError = err;
        finished = true;
        notify?.();
      },
    });

    opts?.signal?.addEventListener('abort', () => {
      finished = true;
      this.streams.delete(requestId);
      notify?.();
    });

    await this.sendEnvelope(envelope);

    while (!finished || queue.length > 0) {
      if (queue.length > 0) {
        yield queue.shift()!;
        continue;
      }
      await new Promise<void>(res => {
        notify = res;
      });
      notify = null;
    }

    this.streams.delete(requestId);

    if (streamError) throw streamError;
  }

  async isHealthy(): Promise<boolean> {
    try {
      await this.ensureConnected();
      await this.call('openhuman.ping', {}, { signal: AbortSignal.timeout(5000) });
      return true;
    } catch {
      return false;
    }
  }

  async close(): Promise<void> {
    log('[tunnel] close channelId=%s', this.channelId);
    this.rejectAllPending(new Error('[tunnel] transport closed'));
    this.socket?.disconnect();
    this.socket = null;
    this._connectPromise = null;
    this.sessionKey = null;
  }

  private rejectAllPending(err: Error): void {
    for (const [, pending] of this.pending) {
      clearTimeout(pending.timeoutId);
      pending.reject(err);
    }
    this.pending.clear();
    for (const [, stream] of this.streams) {
      stream.error(err);
    }
    this.streams.clear();
  }
}
