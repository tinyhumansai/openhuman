import { beforeEach, describe, expect, it, vi } from 'vitest';

import { createCoreSocket } from '../coreSocket';

const hoisted = vi.hoisted(() => ({ ioMock: vi.fn(() => ({ on: vi.fn(), id: 'mock-sid' })) }));

vi.mock('socket.io-client', () => ({ io: hoisted.ioMock }));

const ioMock = hoisted.ioMock;

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
