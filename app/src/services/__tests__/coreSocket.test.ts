import { beforeEach, describe, expect, it, vi } from 'vitest';

import { connectCoreSocket, createCoreSocket } from '../coreSocket';

const hoisted = vi.hoisted(() => ({
  ioMock: vi.fn(() => ({ on: vi.fn(), id: 'mock-sid' })),
  getCoreRpcTokenMock: vi.fn(async (): Promise<string | null> => 'mock-core-bearer'),
}));

vi.mock('socket.io-client', () => ({ io: hoisted.ioMock }));
vi.mock('../coreRpcClient', () => ({ getCoreRpcToken: hoisted.getCoreRpcTokenMock }));

const ioMock = hoisted.ioMock;
const getCoreRpcTokenMock = hoisted.getCoreRpcTokenMock;

describe('createCoreSocket', () => {
  beforeEach(() => {
    ioMock.mockClear();
  });

  it('passes the core bearer through the auth payload', () => {
    createCoreSocket('http://127.0.0.1:7788', { coreToken: 'core-bearer-xyz' });
    expect(ioMock).toHaveBeenCalledTimes(1);
    const call = ioMock.mock.calls[0] as unknown as [string, { auth: { token: string } }];
    expect(call[0]).toBe('http://127.0.0.1:7788');
    expect(call[1].auth.token).toBe('core-bearer-xyz');
  });

  it('substitutes empty string when no core token is available', () => {
    createCoreSocket('http://127.0.0.1:7788', { coreToken: null });
    const call = ioMock.mock.calls[0] as unknown as [string, { auth: { token: string } }];
    expect(call[1].auth.token).toBe('');
  });

  it('merges authExtras alongside the token slot', () => {
    createCoreSocket('http://127.0.0.1:7788', {
      coreToken: 'core',
      authExtras: { session: 'jwt-abc' },
    });
    const call = ioMock.mock.calls[0] as unknown as [
      string,
      { auth: { token: string; session: string } },
    ];
    expect(call[1].auth.token).toBe('core');
    expect(call[1].auth.session).toBe('jwt-abc');
  });

  it('honours overrides without dropping the auth payload', () => {
    createCoreSocket('http://127.0.0.1:7788', {
      coreToken: 'core',
      overrides: { reconnectionAttempts: 5, forceNew: false, timeout: 4000 },
    });
    const call = ioMock.mock.calls[0] as unknown as [
      string,
      { auth: { token: string }; reconnectionAttempts: number; forceNew: boolean; timeout: number },
    ];
    const opts = call[1];
    expect(opts.auth.token).toBe('core');
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
  });

  it('resolves baseUrl + core token then opens the socket', async () => {
    const getBaseUrl = vi.fn().mockResolvedValue('http://127.0.0.1:7788');
    const socket = await connectCoreSocket({ getBaseUrl });
    expect(socket).not.toBeNull();
    expect(getBaseUrl).toHaveBeenCalledTimes(1);
    expect(getCoreRpcTokenMock).toHaveBeenCalledTimes(1);
    expect(ioMock).toHaveBeenCalledTimes(1);
    const call = ioMock.mock.calls[0] as unknown as [string, { auth: { token: string } }];
    expect(call[0]).toBe('http://127.0.0.1:7788');
    expect(call[1].auth.token).toBe('mock-core-bearer');
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
      { auth: { token: string; session: string }; reconnectionAttempts: number },
    ];
    expect(call[1].auth.session).toBe('jwt-xyz');
    expect(call[1].reconnectionAttempts).toBe(7);
  });

  it('passes empty token through when getCoreRpcToken resolves to null', async () => {
    getCoreRpcTokenMock.mockResolvedValueOnce(null);
    const getBaseUrl = vi.fn().mockResolvedValue('http://127.0.0.1:7788');
    await connectCoreSocket({ getBaseUrl });
    const call = ioMock.mock.calls[0] as unknown as [string, { auth: { token: string } }];
    expect(call[1].auth.token).toBe('');
  });
});
