import debug from 'debug';
import { describe, expect, it } from 'vitest';

import { CloudHttpTransport } from './CloudHttpTransport';
import { LanHttpTransport } from './LanHttpTransport';
import type { ConnectionProfile } from './profileStore';
import { createTransportManager } from './TransportManager';

/**
 * A connection profile's `rpcUrl` is stored verbatim — `normalizeRpcUrl` keeps
 * userinfo, query and hash — so it can carry `user:pass@` or `?token=`. That is
 * exactly what `redactRpcUrlForLog` exists for, and what its own test pins.
 *
 * These transports logged the raw URL at construction time. `transport:cloud`
 * was already careful with the bearer token (it reports presence, not value),
 * which is what makes the URL beside it the odd one out.
 */

const SECRETS = ['HUNTER2', 'SUPERSECRET'] as const;
const SECRET_URL = 'https://svc:HUNTER2@core.example.com/rpc?token=SUPERSECRET#/tok';
const SAFE_PART = 'https://core.example.com/rpc';

/** Collect everything the `debug` namespaces emit while `fn` runs. */
function captureDebug(fn: () => void): string {
  const lines: string[] = [];
  const previous = debug.disable();
  const previousLog = debug.log;
  debug.enable('transport:*');
  debug.log = (...args: unknown[]) => {
    lines.push(args.map(String).join(' '));
  };
  try {
    fn();
  } finally {
    debug.log = previousLog;
    debug.disable();
    if (previous) debug.enable(previous);
  }
  return lines.join('\n');
}

function expectNoSecrets(output: string) {
  for (const secret of SECRETS) {
    expect(output).not.toContain(secret);
  }
}

function profile(kind: 'cloud' | 'lan'): ConnectionProfile {
  return {
    id: `p-${kind}`,
    kind,
    rpcUrl: SECRET_URL,
    sessionToken: 'session-value',
  } as unknown as ConnectionProfile;
}

describe('transport construction logging', () => {
  it('CloudHttpTransport does not log rpcUrl credentials', () => {
    const output = captureDebug(() => {
      new CloudHttpTransport(SECRET_URL, 'bearer-value');
    });
    expectNoSecrets(output);
    expect(output).toContain(SAFE_PART);
    // The bearer value was never logged and must stay that way.
    expect(output).not.toContain('bearer-value');
  });

  it('LanHttpTransport does not log rpcUrl credentials', () => {
    const output = captureDebug(() => {
      new LanHttpTransport(SECRET_URL);
    });
    expectNoSecrets(output);
    expect(output).toContain(SAFE_PART);
  });

  it('TransportManager does not log rpcUrl credentials when selecting cloud', async () => {
    let selected: Promise<unknown> | undefined;
    const output = captureDebug(() => {
      selected = createTransportManager(profile('cloud')).getTransport();
    });
    await selected;
    expectNoSecrets(output);
  });

  it('TransportManager does not log rpcUrl credentials when selecting lan', async () => {
    let selected: Promise<unknown> | undefined;
    const output = captureDebug(() => {
      selected = createTransportManager(profile('lan')).getTransport();
    });
    await selected;
    expectNoSecrets(output);
  });
});
