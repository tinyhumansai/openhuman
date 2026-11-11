import { beforeEach, describe, expect, it, vi } from 'vitest';

import { connectCoreSocket, createCoreSocket, lastHandshakeToken } from '../coreSocket';

const hoisted = vi.hoisted(() => ({
  ioMock: vi.fn(() => ({ on: vi.fn(), id: 'mock-sid' })),
  getCoreRpcTokenMock: vi.fn(async (): Promise<string | null> => 'mock-core-bearer'),
  resolveCoreSocketEndpointMock: vi.fn(
    async (
      baseUrl: string
    ): Promise<{ baseUrl: string; path: string; transports?: Array<'websocket' | 'polling'> }> => ({
      baseUrl,
      path: '/socket.io/',
    })
  ),
}));

vi.mock('socket.io-client', () => ({ io: hoisted.ioMock }));
vi.mock('../coreRpcClient', () => ({
  getCoreRpcToken: hoisted.getCoreRpcTokenMock,
  resolveCoreSocketEndpoint: hoisted.resolveCoreSocketEndpointMock,
}));

const ioMock = hoisted.ioMock;
const getCoreRpcTokenMock = hoisted.getCoreRpcTokenMock;
const resolveCoreSocketEndpointMock = hoisted.resolveCoreSocketEndpointMock;

/** Runs the `auth` callback the way socket.io-client does on each handshake. */
function handshake(auth: unknown): Promise<Record<string, unknown>> {
  return new Promise(resolve =>
    (auth as (cb: (data: Record<string, unknown>) => void) => void)(resolve)
  );
}

describe('createCoreSocket', () => {
  beforeEach(() => {
    ioMock.mockClear();
    // No resolvable bearer: the handshake falls back to the caller's token.
    getCoreRpcTokenMock.mockReset();
    getCoreRpcTokenMock.mockResolvedValue(null);
  });

  it('passes the core bearer through the auth payload', async () => {
    createCoreSocket('http://127.0.0.1:7788', { coreToken: 'core-bearer-xyz' });
    expect(ioMock).toHaveBeenCalledTimes(1);
    const call = ioMock.mock.calls[0] as unknown as [string, { auth: unknown }];
    expect(call[0]).toBe('http://127.0.0.1:7788');
    expect((await handshake(call[1].auth)).token).toBe('core-bearer-xyz');
  });

  it('substitutes empty string when no core token is available', async () => {
    createCoreSocket('http://127.0.0.1:7788', { coreToken: null });
    const call = ioMock.mock.calls[0] as unknown as [string, { auth: unknown }];
    expect((await handshake(call[1].auth)).token).toBe('');
  });

  it('merges authExtras alongside the token slot', async () => {
    createCoreSocket('http://127.0.0.1:7788', {
      coreToken: 'core',
      authExtras: { session: 'jwt-abc' },
    });
    const call = ioMock.mock.calls[0] as unknown as [string, { auth: unknown }];
    const payload = await handshake(call[1].auth);
    expect(payload.token).toBe('core');
    expect(payload.session).toBe('jwt-abc');
  });

  it('re-reads the core bearer on every handshake, so a reconnect after a core restart uses the new one', async () => {
    createCoreSocket('http://127.0.0.1:7788', { coreToken: 'launch-1-bearer' });
    const call = ioMock.mock.calls[0] as unknown as [string, { auth: unknown }];
    getCoreRpcTokenMock.mockResolvedValue('launch-1-bearer');
    expect((await handshake(call[1].auth)).token).toBe('launch-1-bearer');
    getCoreRpcTokenMock.mockResolvedValue('launch-2-bearer');
    expect((await handshake(call[1].auth)).token).toBe('launch-2-bearer');
  });

  it('falls back to its own bearer when resolveToken declines (another core is active)', async () => {
    const socket = createCoreSocket('http://127.0.0.1:7788', {
      coreToken: 'core-a-bearer',
      resolveToken: async () => null,
    });
    getCoreRpcTokenMock.mockResolvedValue('core-b-bearer');
    const call = ioMock.mock.calls[0] as unknown as [string, { auth: unknown }];
    expect((await handshake(call[1].auth)).token).toBe('core-a-bearer');
    expect(lastHandshakeToken(socket as never)).toBe('core-a-bearer');
  });

  it('honours overrides without dropping the auth payload', async () => {
    createCoreSocket('http://127.0.0.1:7788', {
      coreToken: 'core',
      overrides: { reconnectionAttempts: 5, forceNew: false, timeout: 4000 },
    });
    const call = ioMock.mock.calls[0] as unknown as [
      string,
      { auth: unknown; reconnectionAttempts: number; forceNew: boolean; timeout: number },
    ];
    const opts = call[1];
    expect((await handshake(opts.auth)).token).toBe('core');
    expect(opts.reconnectionAttempts).toBe(5);
    expect(opts.forceNew).toBe(false);
    expect(opts.timeout).toBe(4000);
  });
});

describe('connectCoreSocket', () => {
  beforeEach(() => {
    ioMock.mockClear();
    getCoreRpcTokenMock.mockReset();
    getCoreRpcTokenMock.mockResolvedValue('mock-core-bearer');
    resolveCoreSocketEndpointMock.mockReset();
    resolveCoreSocketEndpointMock.mockImplementation(async baseUrl => ({
      baseUrl,
      path: '/socket.io/',
    }));
  });

  it('resolves baseUrl + core token then opens the socket', async () => {
    const getBaseUrl = vi.fn().mockResolvedValue('http://127.0.0.1:7788');
    const socket = await connectCoreSocket({ getBaseUrl });
    expect(socket).not.toBeNull();
    expect(getBaseUrl).toHaveBeenCalledTimes(1);
    expect(getCoreRpcTokenMock).toHaveBeenCalledTimes(1);
    expect(ioMock).toHaveBeenCalledTimes(1);
    const call = ioMock.mock.calls[0] as unknown as [string, { auth: unknown }];
    expect(call[0]).toBe('http://127.0.0.1:7788');
    expect((await handshake(call[1].auth)).token).toBe('mock-core-bearer');
  });

  it('short-circuits to null when disposed flips before token resolves', async () => {
    let disposed = false;
    const getBaseUrl = vi.fn().mockImplementation(async () => {
      disposed = true;
      return 'http://127.0.0.1:7788';
    });
    const socket = await connectCoreSocket({ getBaseUrl, isDisposed: () => disposed });
    expect(socket).toBeNull();
    expect(getCoreRpcTokenMock).not.toHaveBeenCalled();
    expect(ioMock).not.toHaveBeenCalled();
  });

  it('short-circuits to null when disposed flips between token and connect', async () => {
    let disposed = false;
    const getBaseUrl = vi.fn().mockResolvedValue('http://127.0.0.1:7788');
    getCoreRpcTokenMock.mockImplementation(async () => {
      disposed = true;
      return 'mock-core-bearer';
    });
    const socket = await connectCoreSocket({ getBaseUrl, isDisposed: () => disposed });
    expect(socket).toBeNull();
    expect(ioMock).not.toHaveBeenCalled();
  });

  it('forwards authExtras + overrides into the underlying io() call', async () => {
    const getBaseUrl = vi.fn().mockResolvedValue('http://127.0.0.1:7788');
    await connectCoreSocket({
      getBaseUrl,
      authExtras: { session: 'jwt-xyz' },
      overrides: { reconnectionAttempts: 7 },
    });
    const call = ioMock.mock.calls[0] as unknown as [
      string,
      { auth: unknown; reconnectionAttempts: number },
    ];
    expect((await handshake(call[1].auth)).session).toBe('jwt-xyz');
    expect(call[1].reconnectionAttempts).toBe(7);
  });

  it('passes empty token through when getCoreRpcToken resolves to null', async () => {
    getCoreRpcTokenMock.mockResolvedValue(null);
    const getBaseUrl = vi.fn().mockResolvedValue('http://127.0.0.1:7788');
    await connectCoreSocket({ getBaseUrl });
    const call = ioMock.mock.calls[0] as unknown as [string, { auth: unknown }];
    expect((await handshake(call[1].auth)).token).toBe('');
  });

  it('uses the private-LAN shell relay and forces WebSocket transport', async () => {
    resolveCoreSocketEndpointMock.mockResolvedValueOnce({
      baseUrl: 'http://127.0.0.1:40000',
      path: '/secret/socket.io/',
      transports: ['websocket'],
    });
    await connectCoreSocket({
      getBaseUrl: async () => 'http://192.168.1.74:7788',
      overrides: { transports: ['polling'] },
    });
    const call = ioMock.mock.calls[0] as unknown as [
      string,
      { path: string; transports: string[] },
    ];
    expect(call[0]).toBe('http://127.0.0.1:40000');
    expect(call[1].path).toBe('/secret/socket.io/');
    expect(call[1].transports).toEqual(['websocket']);
  });
});
