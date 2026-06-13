/**
 * Unit tests for TunnelTransport.
 *
 * We mock socket.io-client so no real network connection is made.
 * Each test gets a fresh socket mock via the module factory pattern.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  base64urlEncode,
  deriveSessionKeys,
  deriveSharedSecret,
  generateKeypair,
  open,
  ReplayTracker,
  seal,
  TunnelCipher,
  type TunnelKeypair,
} from '../../lib/tunnel/crypto';

// -- socket mock factory -------------------------------------------------------

// The mock must be registered before the module under test is imported, but
// we need fresh state per test. We use module-level mutable objects the
// factory closure captures.

let _handlers: Map<string, (...args: unknown[]) => void> = new Map();
let _emitSpy = vi.fn();
let _disconnectSpy = vi.fn();

vi.mock('socket.io-client', () => ({
  io: () => ({
    on: (event: string, cb: (...args: unknown[]) => void) => {
      _handlers.set(event, cb);
    },
    emit: (...args: unknown[]) => _emitSpy(...args),
    disconnect: () => _disconnectSpy(),
    connected: true,
  }),
}));

// Import AFTER vi.mock is hoisted.
const { TunnelTransport } = await import('./TunnelTransport');

// -- helpers ------------------------------------------------------------------

function resetSocket() {
  _handlers = new Map();
  _emitSpy = vi.fn();
  _disconnectSpy = vi.fn();
}

function fire(event: string, ...args: unknown[]) {
  _handlers.get(event)?.(...args);
}

async function connectTransport(
  transport: InstanceType<typeof TunnelTransport>
): Promise<TunnelCipher> {
  const connectP = (transport as unknown as { ensureConnected(): Promise<void> }).ensureConnected();
  // Flush: give socket.on a chance to register.
  await Promise.resolve();
  fire('connect');
  await Promise.resolve();
  fire('tunnel:connected');
  await Promise.resolve();

  type HandshakeInternals = {
    staticDhKey: Uint8Array | null;
    clientEphemeralKeypair: TunnelKeypair | null;
  };
  const internals = transport as unknown as HandshakeInternals;
  expect(internals.staticDhKey).toBeTruthy();
  expect(internals.clientEphemeralKeypair).toBeTruthy();

  const serverEphemeral = generateKeypair();
  const keys = deriveSessionKeys(
    internals.staticDhKey!,
    deriveSharedSecret(serverEphemeral.secretKey, internals.clientEphemeralKeypair!.publicKey),
    internals.clientEphemeralKeypair!.publicKey,
    serverEphemeral.publicKey
  );
  const ack = new TextEncoder().encode(
    JSON.stringify({
      kind: 'handshake_ack',
      server_ephemeral_pubkey: base64urlEncode(serverEphemeral.publicKey),
    })
  );
  fire('tunnel:frame', { payload: base64urlEncode(seal(internals.staticDhKey!, ack)) });
  await connectP;
  return new TunnelCipher('server', keys);
}

function coreB64(kp: ReturnType<typeof generateKeypair>) {
  return base64urlEncode(kp.publicKey);
}

// -- tests --------------------------------------------------------------------

beforeEach(() => {
  resetSocket();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('TunnelTransport', () => {
  it('emits tunnel:connect with channelId + role on connect', async () => {
    const coreKp = generateKeypair();
    const channelId = 'CHAN_001';
    const transport = new TunnelTransport('http://backend', channelId, coreB64(coreKp), 'tok');

    await connectTransport(transport);

    const connectCall = _emitSpy.mock.calls.find(([ev]) => ev === 'tunnel:connect');
    expect(connectCall).toBeTruthy();
    expect(connectCall![1]).toMatchObject({ channelId, role: 'client', token: 'tok' });

    // Handshake frame should have been sent.
    const frameCall = _emitSpy.mock.calls.find(([ev]) => ev === 'tunnel:frame');
    expect(frameCall).toBeTruthy();

    await transport.close();
  });

  it('emits a session token field for reconnect authentication', async () => {
    const coreKp = generateKeypair();
    const transport = new TunnelTransport(
      'http://backend',
      'CHAN_SESSION',
      coreB64(coreKp),
      'sess_tok',
      undefined,
      'session'
    );

    await connectTransport(transport);

    const connectCall = _emitSpy.mock.calls.find(([ev]) => ev === 'tunnel:connect');
    expect(connectCall![1]).toMatchObject({
      channelId: 'CHAN_SESSION',
      token: 'sess_tok',
      sessionToken: 'sess_tok',
    });
    expect((connectCall![1] as { pairingToken?: string }).pairingToken).toBeUndefined();

    await transport.close();
  });

  it('rejects pending calls when close() is called', async () => {
    const coreKp = generateKeypair();
    const transport = new TunnelTransport('http://backend', 'CHAN_002', coreB64(coreKp), 'tok');

    await connectTransport(transport);

    // Queue a call.
    const callP = transport.call('openhuman.ping', {});

    // Close immediately — pending call should reject.
    await transport.close();

    await expect(callP).rejects.toThrow();
  }, 5000);

  it('replay rejection: duplicate encrypted frames are rejected', () => {
    const kp = generateKeypair();
    const other = generateKeypair();
    const key = deriveSharedSecret(kp.secretKey, other.publicKey);
    const tracker = new ReplayTracker();

    const plain = new TextEncoder().encode(
      '{"requestId":"r1","kind":"response","seq":0,"payload":null}'
    );
    const frame = seal(key, plain);

    // First open: ok.
    const first = open(key, frame, tracker);
    expect(Array.from(first)).toEqual(Array.from(plain));

    // Second open of same frame: replayed nonce.
    expect(() => open(key, frame, tracker)).toThrow(/replayed nonce/i);
  });

  it('rejects the connect promise on tunnel:error', async () => {
    const coreKp = generateKeypair();
    const transport = new TunnelTransport('http://backend', 'CHAN_003', coreB64(coreKp), 'tok');

    const connectP = (
      transport as unknown as { ensureConnected(): Promise<void> }
    ).ensureConnected();
    await Promise.resolve();
    fire('connect');
    await Promise.resolve();
    // Fire tunnel:error instead of tunnel:connected.
    fire('tunnel:error', 'unauthorized');

    await expect(connectP).rejects.toThrow(/server error|unauthorized/i);
  }, 5000);

  it('rejects the connect promise when the handshake ack times out', async () => {
    vi.useFakeTimers();
    const coreKp = generateKeypair();
    const transport = new TunnelTransport('http://backend', 'CHAN_TIMEOUT', coreB64(coreKp), 'tok');

    const connectP = (
      transport as unknown as { ensureConnected(): Promise<void> }
    ).ensureConnected();
    await Promise.resolve();
    fire('connect');
    await Promise.resolve();
    fire('tunnel:connected');
    await Promise.resolve();

    vi.advanceTimersByTime(10_000);
    await expect(connectP).rejects.toThrow(/handshake ack timed out/i);

    await transport.close();
  });

  it('rejects the connect promise when the handshake ack cannot be opened', async () => {
    const coreKp = generateKeypair();
    const transport = new TunnelTransport(
      'http://backend',
      'CHAN_BAD_ACK_OPEN',
      coreB64(coreKp),
      'tok'
    );

    const connectP = (
      transport as unknown as { ensureConnected(): Promise<void> }
    ).ensureConnected();
    await Promise.resolve();
    fire('connect');
    await Promise.resolve();
    fire('tunnel:connected');
    await Promise.resolve();

    fire('tunnel:frame', { payload: base64urlEncode(new Uint8Array([1, 2, 3, 4])) });
    await expect(connectP).rejects.toThrow();

    await transport.close();
  });

  it('rejects the connect promise when the handshake ack is not JSON', async () => {
    const coreKp = generateKeypair();
    const transport = new TunnelTransport(
      'http://backend',
      'CHAN_BAD_ACK_JSON',
      coreB64(coreKp),
      'tok'
    );

    const connectP = (
      transport as unknown as { ensureConnected(): Promise<void> }
    ).ensureConnected();
    await Promise.resolve();
    fire('connect');
    await Promise.resolve();
    fire('tunnel:connected');
    await Promise.resolve();

    type HandshakeInternals = { staticDhKey: Uint8Array | null };
    const internals = transport as unknown as HandshakeInternals;
    expect(internals.staticDhKey).toBeTruthy();

    fire('tunnel:frame', {
      payload: base64urlEncode(seal(internals.staticDhKey!, new TextEncoder().encode('not json'))),
    });
    await expect(connectP).rejects.toThrow();

    await transport.close();
  });

  it('rejects the connect promise when the handshake ack kind is invalid', async () => {
    const coreKp = generateKeypair();
    const transport = new TunnelTransport(
      'http://backend',
      'CHAN_BAD_ACK_KIND',
      coreB64(coreKp),
      'tok'
    );

    const connectP = (
      transport as unknown as { ensureConnected(): Promise<void> }
    ).ensureConnected();
    await Promise.resolve();
    fire('connect');
    await Promise.resolve();
    fire('tunnel:connected');
    await Promise.resolve();

    type HandshakeInternals = { staticDhKey: Uint8Array | null };
    const internals = transport as unknown as HandshakeInternals;
    expect(internals.staticDhKey).toBeTruthy();

    fire('tunnel:frame', {
      payload: base64urlEncode(
        seal(internals.staticDhKey!, new TextEncoder().encode(JSON.stringify({ kind: 'nope' })))
      ),
    });
    await expect(connectP).rejects.toThrow(/invalid handshake ack/i);

    await transport.close();
  });

  it('rejects an in-flight handshake when close() is called', async () => {
    const coreKp = generateKeypair();
    const transport = new TunnelTransport(
      'http://backend',
      'CHAN_CLOSE_HANDSHAKE',
      coreB64(coreKp),
      'tok'
    );

    const connectP = (
      transport as unknown as { ensureConnected(): Promise<void> }
    ).ensureConnected();
    await Promise.resolve();
    fire('connect');
    await Promise.resolve();
    fire('tunnel:connected');
    await Promise.resolve();

    await transport.close();

    await expect(connectP).rejects.toThrow(/transport closed/i);
  });

  it('resolves call() when a matching encrypted response frame arrives', async () => {
    const coreKp = generateKeypair();
    const transport = new TunnelTransport('http://backend', 'CHAN_004', coreB64(coreKp), 'tok');

    const serverCipher = await connectTransport(transport);

    const callP = transport.call<{ pong: number }>('openhuman.ping', { who: 'me' });

    // Wait for the call to register and send its frame.
    await Promise.resolve();
    await Promise.resolve();

    const { chunk } = await import('../../lib/tunnel/framing');

    // Pull the device pubkey out of the handshake frame the client sent.
    const handshakeCall = _emitSpy.mock.calls.find(([ev]) => ev === 'tunnel:frame');
    expect(handshakeCall).toBeTruthy();

    type Internals = { pending: Map<string, unknown> };
    const internals = transport as unknown as Internals;

    // Wait until the pending request is registered.
    for (let i = 0; i < 10 && internals.pending.size === 0; i++) {
      await Promise.resolve();
    }
    const [requestId] = Array.from(internals.pending.keys()) as string[];
    expect(requestId).toBeTruthy();

    // Build a response envelope, chunk it, encrypt each chunk, and feed back
    // via the tunnel:frame handler.
    const envelope = { requestId, kind: 'response' as const, seq: 0, payload: { pong: 42 } };
    for (const raw of chunk(envelope)) {
      const encrypted = serverCipher.seal(raw);
      fire('tunnel:frame', { payload: base64urlEncode(encrypted) });
    }

    await expect(callP).resolves.toEqual({ pong: 42 });

    await transport.close();
  }, 10000);

  it('routes error envelopes back to the matching pending call', async () => {
    const coreKp = generateKeypair();
    const transport = new TunnelTransport('http://backend', 'CHAN_005', coreB64(coreKp), 'tok');
    const serverCipher = await connectTransport(transport);

    const callP = transport.call('openhuman.fail', {});
    await Promise.resolve();
    await Promise.resolve();

    const { chunk } = await import('../../lib/tunnel/framing');
    type Internals = { pending: Map<string, unknown> };
    const internals = transport as unknown as Internals;
    for (let i = 0; i < 10 && internals.pending.size === 0; i++) {
      await Promise.resolve();
    }
    const [requestId] = Array.from(internals.pending.keys()) as string[];

    const envelope = { requestId, kind: 'error' as const, seq: 0, payload: 'tunnel exploded' };
    for (const raw of chunk(envelope)) {
      fire('tunnel:frame', { payload: base64urlEncode(serverCipher.seal(raw)) });
    }

    await expect(callP).rejects.toThrow('tunnel exploded');
    await transport.close();
  }, 10000);

  it('ignores incoming frames missing a payload field', async () => {
    const coreKp = generateKeypair();
    const transport = new TunnelTransport('http://backend', 'CHAN_006', coreB64(coreKp), 'tok');
    await connectTransport(transport);

    // Should not throw, should not affect any pending state.
    fire('tunnel:frame', { not_payload: 'oops' });
    fire('tunnel:frame', { payload: 42 });
    fire('tunnel:frame', null);

    await transport.close();
  });

  it('ignores frames that arrive before the session cipher is set', async () => {
    const coreKp = generateKeypair();
    const transport = new TunnelTransport('http://backend', 'CHAN_007', coreB64(coreKp), 'tok');

    // Start connect but don't complete handshake.
    void (transport as unknown as { ensureConnected(): Promise<void> }).ensureConnected();
    await Promise.resolve();
    fire('connect');
    // (no tunnel:connected → no handshake → session cipher stays null)

    // Frame arrives early — should be silently dropped.
    fire('tunnel:frame', { payload: 'AAAAAAA' });

    // No assertion needed beyond "no throw". Force the connect promise to
    // settle so vitest doesn't complain about leaks.
    await transport.close();
  });

  it('isHealthy returns false when the underlying connect rejects', async () => {
    const coreKp = generateKeypair();
    const transport = new TunnelTransport('http://backend', 'CHAN_008', coreB64(coreKp), 'tok');
    const healthyP = transport.isHealthy();
    await Promise.resolve();
    // Surface a connect_error before tunnel:connected — connect rejects.
    fire('connect_error', new Error('refused'));
    await expect(healthyP).resolves.toBe(false);
  });

  it('disconnect resets the session cipher and connect promise', async () => {
    const coreKp = generateKeypair();
    const transport = new TunnelTransport('http://backend', 'CHAN_009', coreB64(coreKp), 'tok');
    await connectTransport(transport);

    type Internals = { cipher: TunnelCipher | null; _connectPromise: Promise<void> | null };
    const internals = transport as unknown as Internals;
    expect(internals.cipher).toBeTruthy();

    fire('disconnect', 'transport close');
    expect(internals.cipher).toBeNull();
    expect(internals._connectPromise).toBeNull();

    await transport.close();
  });
});
