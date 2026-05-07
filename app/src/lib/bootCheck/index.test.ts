/**
 * Unit tests for the boot-check orchestrator.
 *
 * Uses the injectable transport so no real Tauri IPC or HTTP calls are made.
 */
import { describe, expect, it, vi } from 'vitest';

import { type BootCheckResult, type BootCheckTransport, runBootCheck } from './index';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Build a minimal transport stub for tests. */
function makeTransport(overrides?: Partial<BootCheckTransport>): BootCheckTransport {
  return { callRpc: vi.fn(), invokeCmd: vi.fn().mockResolvedValue(undefined), ...overrides };
}

/**
 * Build a callRpc mock that answers specific methods.
 *
 * `responses` maps method-name → resolved value (or Error to reject with).
 */
function rpcResponder(responses: Record<string, unknown>): BootCheckTransport['callRpc'] {
  return vi.fn(async (method: string) => {
    if (method in responses) {
      const val = responses[method];
      if (val instanceof Error) throw val;
      return val;
    }
    throw new Error(`Unexpected RPC call: ${method}`);
  }) as BootCheckTransport['callRpc'];
}

// ---------------------------------------------------------------------------
// Local mode tests
// ---------------------------------------------------------------------------

describe('runBootCheck — local mode', () => {
  it('returns match when ping succeeds, no daemon, versions match', async () => {
    const appVersion = (await import('../../utils/config')).APP_VERSION;

    const transport = makeTransport({
      callRpc: rpcResponder({
        'openhuman.ping': {},
        'openhuman.service_status': { installed: false, running: false },
        'openhuman.update_version': { version_info: { version: appVersion } },
      }),
    });

    const result = await runBootCheck({ kind: 'local' }, transport);
    expect(result).toEqual({ kind: 'match' });
  });

  it('returns daemonDetected when service_status shows installed=true', async () => {
    const appVersion = (await import('../../utils/config')).APP_VERSION;

    const transport = makeTransport({
      callRpc: rpcResponder({
        'openhuman.ping': {},
        'openhuman.service_status': { installed: true, running: false },
        'openhuman.update_version': { version_info: { version: appVersion } },
      }),
    });

    const result = await runBootCheck({ kind: 'local' }, transport);
    expect(result).toEqual({ kind: 'daemonDetected' });
  });

  it('returns daemonDetected when service_status shows running=true', async () => {
    const transport = makeTransport({
      callRpc: rpcResponder({
        'openhuman.ping': {},
        'openhuman.service_status': { installed: false, running: true },
        'openhuman.update_version': { version_info: { version: 'x' } },
      }),
    });

    const result = await runBootCheck({ kind: 'local' }, transport);
    expect(result).toEqual({ kind: 'daemonDetected' });
  });

  it('returns outdatedLocal when core version differs from app version', async () => {
    const transport = makeTransport({
      callRpc: rpcResponder({
        'openhuman.ping': {},
        'openhuman.service_status': { installed: false, running: false },
        'openhuman.update_version': { version_info: { version: '0.0.0-different' } },
      }),
    });

    const result = await runBootCheck({ kind: 'local' }, transport);
    expect(result).toEqual({ kind: 'outdatedLocal' });
  });

  it('returns noVersionMethod when update_version returns -32601', async () => {
    const transport = makeTransport({
      callRpc: rpcResponder({
        'openhuman.ping': {},
        'openhuman.service_status': { installed: false, running: false },
        'openhuman.update_version': new Error('JSON-RPC error -32601 Method not found'),
      }),
    });

    const result = await runBootCheck({ kind: 'local' }, transport);
    expect(result).toEqual({ kind: 'noVersionMethod' });
  });

  it('returns noVersionMethod on "method not found" text variant', async () => {
    const transport = makeTransport({
      callRpc: rpcResponder({
        'openhuman.ping': {},
        'openhuman.service_status': { installed: false, running: false },
        'openhuman.update_version': new Error('method not found'),
      }),
    });

    const result = await runBootCheck({ kind: 'local' }, transport);
    expect(result).toEqual({ kind: 'noVersionMethod' });
  });

  it('returns unreachable when start_core_process invoke fails', async () => {
    const transport = makeTransport({
      invokeCmd: vi.fn().mockRejectedValue(new Error('process launch failed')),
      callRpc: vi.fn(),
    });

    const result = await runBootCheck({ kind: 'local' }, transport);
    expect(result.kind).toBe('unreachable');
  });

  it('returns unreachable when ping never succeeds', async () => {
    // Provide a fast-cycling callRpc that always fails ping
    const callRpc = vi.fn().mockRejectedValue(new Error('ECONNREFUSED'));
    const transport = makeTransport({ callRpc });

    // Override setTimeout to avoid real waiting — tick forward immediately
    vi.useFakeTimers();
    const promise = runBootCheck({ kind: 'local' }, transport);
    // Drain all pending micro-tasks + setTimeout callbacks
    await vi.runAllTimersAsync();
    const result = await promise;
    vi.useRealTimers();

    expect(result.kind).toBe('unreachable');
  });
});

// ---------------------------------------------------------------------------
// Cloud mode tests
// ---------------------------------------------------------------------------

describe('runBootCheck — cloud mode', () => {
  it('returns match when cloud core version matches', async () => {
    const appVersion = (await import('../../utils/config')).APP_VERSION;

    const transport = makeTransport({
      callRpc: rpcResponder({
        'openhuman.update_version': { version_info: { version: appVersion } },
      }),
    });

    const result = await runBootCheck(
      { kind: 'cloud', url: 'https://core.example.com/rpc' },
      transport
    );
    expect(result).toEqual({ kind: 'match' });
  });

  it('returns outdatedCloud when version differs', async () => {
    const transport = makeTransport({
      callRpc: rpcResponder({
        'openhuman.update_version': { version_info: { version: '0.0.0-old' } },
      }),
    });

    const result = await runBootCheck(
      { kind: 'cloud', url: 'https://core.example.com/rpc' },
      transport
    );
    expect(result).toEqual({ kind: 'outdatedCloud' });
  });

  it('returns noVersionMethod when cloud core returns -32601', async () => {
    const transport = makeTransport({
      callRpc: rpcResponder({ 'openhuman.update_version': new Error('-32601 Method not found') }),
    });

    const result = await runBootCheck(
      { kind: 'cloud', url: 'https://core.example.com/rpc' },
      transport
    );
    expect(result).toEqual({ kind: 'noVersionMethod' });
  });

  it('returns unreachable on network failure', async () => {
    const transport = makeTransport({
      callRpc: vi.fn().mockRejectedValue(new Error('Network unreachable')),
    });

    const result = await runBootCheck(
      { kind: 'cloud', url: 'https://unreachable.example.com/rpc' },
      transport
    );
    expect(result.kind).toBe('unreachable');
  });
});

// ---------------------------------------------------------------------------
// Unset mode guard
// ---------------------------------------------------------------------------

describe('runBootCheck — unset mode', () => {
  it('returns unreachable when called with unset mode', async () => {
    const transport = makeTransport();
    const result: BootCheckResult = await runBootCheck({ kind: 'unset' }, transport);
    expect(result.kind).toBe('unreachable');
  });
});

// ---------------------------------------------------------------------------
// Edge-case branches surfaced by the diff-coverage gate
// ---------------------------------------------------------------------------

describe('runBootCheck — error and edge branches', () => {
  it('treats service_status throw as "no daemon" and continues', async () => {
    const appVersion = (await import('../../utils/config')).APP_VERSION;

    const transport = makeTransport({
      callRpc: rpcResponder({
        'openhuman.ping': {},
        'openhuman.service_status': new Error('rpc transport blew up'),
        'openhuman.update_version': { version_info: { version: appVersion } },
      }),
    });

    const result = await runBootCheck({ kind: 'local' }, transport);
    expect(result.kind).toBe('match');
  });

  it('treats empty version_info.version as outdatedLocal', async () => {
    const transport = makeTransport({
      callRpc: rpcResponder({
        'openhuman.ping': {},
        'openhuman.service_status': { installed: false, running: false },
        'openhuman.update_version': { version_info: { version: '' } },
      }),
    });

    const result = await runBootCheck({ kind: 'local' }, transport);
    expect(result.kind).toBe('outdatedLocal');
  });

  it('returns unreachable when start_core_process Tauri command fails', async () => {
    const transport: BootCheckTransport = {
      callRpc: vi.fn(),
      invokeCmd: vi.fn().mockRejectedValue(new Error('tauri command not registered')),
    };

    const result = await runBootCheck({ kind: 'local' }, transport);
    expect(result.kind).toBe('unreachable');
    if (result.kind === 'unreachable') {
      expect(result.reason).toContain('Failed to start local core');
    }
  });

  it('returns unreachable when local version check throws repeatedly', async () => {
    let pingCalls = 0;
    const transport: BootCheckTransport = {
      callRpc: vi.fn(async (method: string) => {
        if (method === 'openhuman.ping') {
          pingCalls += 1;
          if (pingCalls === 1) return {};
          throw new Error('subsequent failure');
        }
        if (method === 'openhuman.service_status') {
          return { installed: false, running: false };
        }
        if (method === 'openhuman.update_version') {
          // Generic transport error (no -32601), should map to 'unreachable'.
          throw new Error('connection reset');
        }
        throw new Error(`Unexpected RPC: ${method}`);
      }) as BootCheckTransport['callRpc'],
      invokeCmd: vi.fn().mockResolvedValue(undefined),
    };

    const result = await runBootCheck({ kind: 'local' }, transport);
    expect(result.kind).toBe('unreachable');
  });

  it('refuses to persist an invalid cloud URL', async () => {
    const transport = makeTransport();
    const result = await runBootCheck({ kind: 'cloud', url: 'not a url' }, transport);
    expect(result.kind).toBe('unreachable');
    if (result.kind === 'unreachable') {
      expect(result.reason).toContain('valid URL');
    }
    expect(transport.callRpc).not.toHaveBeenCalled();
  });
});
