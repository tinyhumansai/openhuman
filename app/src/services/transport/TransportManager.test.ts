/**
 * Unit tests for TransportManager race semantics.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { ConnectionProfile } from './profileStore';
import { TransportManager } from './TransportManager';

// -- helpers -----------------------------------------------------------------

function makeProfile(
  kind: ConnectionProfile['kind'],
  overrides: Partial<ConnectionProfile> = {}
): ConnectionProfile {
  return {
    id: 'test-profile',
    label: 'Test',
    kind,
    rpcUrl: kind === 'lan' || kind === 'cloud' ? 'http://localhost:7788/rpc' : undefined,
    channelId: kind === 'tunnel' ? 'CHANNEL001' : undefined,
    corePubkey: kind === 'tunnel' ? 'dGVzdHB1YmtleXRlc3RwdWJrZXl0ZXN0cHVia2V5' : undefined,
    sessionToken: kind === 'tunnel' ? 'tok123' : undefined,
    ...overrides,
  };
}

// -- tests -------------------------------------------------------------------

describe('TransportManager', () => {
  // Stub LanHttpTransport and TunnelTransport constructors.
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('local profile returns LocalTransport', async () => {
    const profile = makeProfile('local');
    const manager = new TransportManager(
      profile,
      () => Promise.resolve('http://localhost:7788/rpc'),
      () => Promise.resolve('tok'),
      'http://backend:3000'
    );
    const t = await manager.getTransport();
    expect(t.kind).toBe('local');
    await manager.close();
  });

  it('lan profile returns LanHttpTransport', async () => {
    const profile = makeProfile('lan');
    const manager = new TransportManager(
      profile,
      () => Promise.resolve(''),
      () => Promise.resolve(null),
      ''
    );
    const t = await manager.getTransport();
    expect(t.kind).toBe('lan-http');
    await manager.close();
  });

  it('cloud profile returns CloudHttpTransport', async () => {
    const profile = makeProfile('cloud');
    const manager = new TransportManager(
      profile,
      () => Promise.resolve(''),
      () => Promise.resolve(null),
      ''
    );
    const t = await manager.getTransport();
    expect(t.kind).toBe('cloud-http');
    await manager.close();
  });

  it('tunnel profile without rpcUrl uses tunnel only', async () => {
    const profile = makeProfile('tunnel', { rpcUrl: undefined });
    const manager = new TransportManager(
      profile,
      () => Promise.resolve(''),
      () => Promise.resolve(null),
      'http://backend:3000'
    );
    const t = await manager.getTransport();
    expect(t.kind).toBe('tunnel');
    await manager.close();
  });

  it('throws when tunnel profile missing channelId', async () => {
    const profile = makeProfile('tunnel', { channelId: undefined });
    const manager = new TransportManager(
      profile,
      () => Promise.resolve(''),
      () => Promise.resolve(null),
      'http://backend:3000'
    );
    await expect(manager.getTransport()).rejects.toThrow(/channelId/);
  });

  it('throws when tunnel profile missing token', async () => {
    const profile = makeProfile('tunnel', { sessionToken: undefined, pairingToken: undefined });
    const manager = new TransportManager(
      profile,
      () => Promise.resolve(''),
      () => Promise.resolve(null),
      'http://backend:3000'
    );
    await expect(manager.getTransport()).rejects.toThrow(/sessionToken|pairingToken/);
  });

  it('reset() clears cached transport and allows re-selection', async () => {
    const profile = makeProfile('local');
    const manager = new TransportManager(
      profile,
      () => Promise.resolve('http://localhost:7788/rpc'),
      () => Promise.resolve('tok'),
      ''
    );
    const t1 = await manager.getTransport();
    await manager.reset();
    const t2 = await manager.getTransport();
    expect(t1.kind).toBe(t2.kind);
  });
});
