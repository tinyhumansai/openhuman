/**
 * LAN-vs-Tunnel race semantics for TransportManager.raceLanAndTunnel (plan.md
 * §4 P1). The sibling TransportManager.test.ts covers profile *selection* but
 * never exercises the race path, despite the docstring describing it. LAN and
 * Tunnel transports are mocked here so each scenario controls who is healthy
 * (and when), and asserts the winner + that the loser is closed.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { ConnectionProfile } from './profileStore';
import { TransportManager } from './TransportManager';

// Mutable health/close hooks the mocked transports read at call time. Declared
// via vi.hoisted so the vi.mock factories (hoisted above imports) can see them.
const h = vi.hoisted(() => ({
  lanHealthy: (): Promise<boolean> => Promise.resolve(false),
  tunnelHealthy: (): Promise<boolean> => Promise.resolve(false),
  lanClose: vi.fn((): Promise<void> => Promise.resolve()),
  tunnelClose: vi.fn((): Promise<void> => Promise.resolve()),
}));

vi.mock('./LanHttpTransport', () => ({
  LanHttpTransport: class {
    readonly kind = 'lan-http';
    isHealthy() {
      return h.lanHealthy();
    }
    close() {
      return h.lanClose();
    }
  },
}));

vi.mock('./TunnelTransport', () => ({
  TunnelTransport: class {
    readonly kind = 'tunnel';
    isHealthy() {
      return h.tunnelHealthy();
    }
    close() {
      return h.tunnelClose();
    }
  },
}));

function tunnelProfile(overrides: Partial<ConnectionProfile> = {}): ConnectionProfile {
  return {
    id: 'race-profile',
    label: 'Race',
    kind: 'tunnel',
    // rpcUrl present → raceLanAndTunnel actually races (not tunnel-only).
    rpcUrl: 'http://192.168.1.5:7788/rpc',
    channelId: 'CHANNEL001',
    corePubkey: 'dGVzdHB1YmtleXRlc3RwdWJrZXl0ZXN0cHVia2V5',
    sessionToken: 'tok123',
    ...overrides,
  };
}

function manager(profile: ConnectionProfile): TransportManager {
  return new TransportManager(
    profile,
    () => Promise.resolve(''),
    () => Promise.resolve(null),
    'http://backend:3000'
  );
}

const immediately = (v: boolean) => () => Promise.resolve(v);
const after = (ms: number, v: boolean) => () =>
  new Promise<boolean>(resolve => setTimeout(() => resolve(v), ms));

describe('TransportManager.raceLanAndTunnel', () => {
  beforeEach(() => {
    h.lanHealthy = immediately(false);
    h.tunnelHealthy = immediately(false);
    h.lanClose.mockClear();
    h.tunnelClose.mockClear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('LAN wins the race and the tunnel is closed', async () => {
    h.lanHealthy = immediately(true);
    h.tunnelHealthy = after(50, true);

    const t = await manager(tunnelProfile()).getTransport();

    expect(t.kind).toBe('lan-http');
    expect(h.tunnelClose).toHaveBeenCalledTimes(1);
    expect(h.lanClose).not.toHaveBeenCalled();
  });

  it('tunnel wins the race and the LAN transport is closed', async () => {
    h.tunnelHealthy = immediately(true);
    h.lanHealthy = after(50, true);

    const t = await manager(tunnelProfile()).getTransport();

    expect(t.kind).toBe('tunnel');
    expect(h.lanClose).toHaveBeenCalledTimes(1);
    expect(h.tunnelClose).not.toHaveBeenCalled();
  });

  it('throws when neither transport becomes healthy', async () => {
    h.lanHealthy = immediately(false);
    h.tunnelHealthy = immediately(false);

    await expect(manager(tunnelProfile()).getTransport()).rejects.toThrow(/all transports failed/);
  });

  it('reset() re-races: LAN wins first, then after reset the tunnel wins', async () => {
    const mgr = manager(tunnelProfile());

    // First race: LAN wins.
    h.lanHealthy = immediately(true);
    h.tunnelHealthy = after(50, false);
    const first = await mgr.getTransport();
    expect(first.kind).toBe('lan-http');

    // LAN later fails → caller resets, and the next race must pick the tunnel
    // (tunnel healthy immediately; LAN's slow-unhealthy result loses the race).
    await mgr.reset();
    h.lanHealthy = after(50, false);
    h.tunnelHealthy = immediately(true);
    const second = await mgr.getTransport();
    expect(second.kind).toBe('tunnel');
  });

  it('caches the winner: a second getTransport does not re-race', async () => {
    h.lanHealthy = immediately(true);
    h.tunnelHealthy = immediately(false);
    const mgr = manager(tunnelProfile());

    const a = await mgr.getTransport();
    const b = await mgr.getTransport();

    expect(a).toBe(b);
    // Tunnel closed exactly once (from the single race), not twice.
    expect(h.tunnelClose).toHaveBeenCalledTimes(1);
  });
});
