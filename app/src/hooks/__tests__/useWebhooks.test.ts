import { renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { buildWebhookEventsUrl, clearCoreRpcTokenCache } from '../../services/coreRpcClient';

// `EventSource` is not implemented in jsdom; install a recording stub so we
// can assert URLs the hook builds + that token rotation actually tears down
// the previous subscription.
class MockEventSource {
  static instances: MockEventSource[] = [];
  url: string;
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  addEventListener = vi.fn();
  close = vi.fn();
  constructor(url: string) {
    this.url = url;
    MockEventSource.instances.push(this);
  }
}

// `vi.mock` calls are hoisted above all `const` declarations, so any
// references the factory closures over must live inside `vi.hoisted` too.
const { mockGetCoreRpcToken, mockGetCoreHttpBaseUrl, sessionTokenRef } = vi.hoisted(() => ({
  mockGetCoreRpcToken: vi.fn<() => Promise<string | null>>(),
  mockGetCoreHttpBaseUrl: vi.fn<() => Promise<string>>(),
  sessionTokenRef: { current: 'session-1' as string | null },
}));

vi.mock('../../services/coreRpcClient', async () => {
  const actual = await vi.importActual<typeof import('../../services/coreRpcClient')>(
    '../../services/coreRpcClient'
  );
  return {
    ...actual,
    getCoreRpcToken: mockGetCoreRpcToken,
    getCoreHttpBaseUrl: mockGetCoreHttpBaseUrl,
  };
});

vi.mock('../../providers/CoreStateProvider', () => ({
  useCoreState: () => ({ snapshot: { sessionToken: sessionTokenRef.current } }),
}));

vi.mock('../../services/api/tunnelsApi', () => ({
  tunnelsApi: {
    getTunnels: vi.fn().mockResolvedValue([]),
    createTunnel: vi.fn(),
    deleteTunnel: vi.fn(),
  },
}));

vi.mock('../../utils/tauriCommands', () => ({
  openhumanWebhooksListLogs: vi.fn().mockResolvedValue({ result: { result: { logs: [] } } }),
  openhumanWebhooksListRegistrations: vi
    .fn()
    .mockResolvedValue({ result: { result: { registrations: [] } } }),
  openhumanWebhooksRegisterEcho: vi.fn(),
  openhumanWebhooksUnregisterEcho: vi.fn(),
}));

describe('buildWebhookEventsUrl', () => {
  it('returns null when token is null', () => {
    expect(buildWebhookEventsUrl('http://localhost:7788', null)).toBeNull();
  });

  it('returns null when token is empty string', () => {
    expect(buildWebhookEventsUrl('http://localhost:7788', '')).toBeNull();
  });

  it('appends ?token=<bearer> for a normal hex token', () => {
    expect(buildWebhookEventsUrl('http://localhost:7788', 'cafebabe')).toBe(
      'http://localhost:7788/events/webhooks?token=cafebabe'
    );
  });

  it('URL-encodes tokens that contain reserved characters', () => {
    expect(buildWebhookEventsUrl('http://localhost:7788', 'a/b+c d')).toBe(
      'http://localhost:7788/events/webhooks?token=a%2Fb%2Bc%20d'
    );
  });
});

describe('useWebhooks SSE auth', () => {
  beforeEach(() => {
    MockEventSource.instances.length = 0;
    sessionTokenRef.current = 'session-1';
    (globalThis as unknown as { EventSource: typeof MockEventSource }).EventSource =
      MockEventSource;
    mockGetCoreRpcToken.mockReset();
    mockGetCoreHttpBaseUrl.mockReset();
    mockGetCoreHttpBaseUrl.mockResolvedValue('http://localhost:7788');
  });

  afterEach(() => {
    delete (globalThis as unknown as { EventSource?: typeof MockEventSource }).EventSource;
  });

  it('skips EventSource when no core RPC token is available', async () => {
    mockGetCoreRpcToken.mockResolvedValue(null);
    const { useWebhooks } = await import('../useWebhooks');

    renderHook(() => useWebhooks());

    // Wait on the observable signal (resolver called + no EventSource ever
    // constructed) rather than a real-time sleep so the test is
    // deterministic on slow CI.
    await waitFor(() => {
      expect(mockGetCoreRpcToken).toHaveBeenCalled();
      expect(MockEventSource.instances).toHaveLength(0);
    });
  });

  it('treats a rejected getCoreRpcToken() as no-token (catch path)', async () => {
    // Token resolver failing must not let the hook open an unauth SSE; it
    // should fall through to the same "no token" branch as a null resolve.
    mockGetCoreRpcToken.mockRejectedValue(new Error('IPC bridge unavailable'));
    const { useWebhooks } = await import('../useWebhooks');

    renderHook(() => useWebhooks());

    await waitFor(() => {
      expect(mockGetCoreRpcToken).toHaveBeenCalled();
      expect(MockEventSource.instances).toHaveLength(0);
    });
  });

  it('constructs EventSource with ?token=<rpc-token> once the token resolves', async () => {
    mockGetCoreRpcToken.mockResolvedValue('rpc-token-1');
    const { useWebhooks } = await import('../useWebhooks');

    renderHook(() => useWebhooks());
    await waitFor(() => expect(MockEventSource.instances).toHaveLength(1));
    expect(MockEventSource.instances[0].url).toBe(
      'http://localhost:7788/events/webhooks?token=rpc-token-1'
    );
  });

  it('closes the old EventSource and opens a new one when the RPC token rotates', async () => {
    mockGetCoreRpcToken.mockResolvedValueOnce('rpc-token-1').mockResolvedValueOnce('rpc-token-2');
    const { useWebhooks } = await import('../useWebhooks');

    const { rerender } = renderHook(() => useWebhooks());
    await waitFor(() => expect(MockEventSource.instances).toHaveLength(1));
    const first = MockEventSource.instances[0];

    // Session-token flip is the FE proxy for "auth state moved" — it makes
    // useWebhooks re-resolve the core RPC token, which (per the mock) now
    // returns the new value.
    sessionTokenRef.current = 'session-2';
    rerender();

    await waitFor(() => expect(MockEventSource.instances).toHaveLength(2));
    expect(first.close).toHaveBeenCalledTimes(1);
    expect(MockEventSource.instances[1].url).toBe(
      'http://localhost:7788/events/webhooks?token=rpc-token-2'
    );
  });

  it('reconnects when restart_core_process invalidates the token cache', async () => {
    mockGetCoreRpcToken.mockResolvedValueOnce('rpc-token-1').mockResolvedValueOnce('rpc-token-2');
    const { useWebhooks } = await import('../useWebhooks');

    renderHook(() => useWebhooks());
    await waitFor(() => expect(MockEventSource.instances).toHaveLength(1));
    const first = MockEventSource.instances[0];

    // Simulate restart_core_process completing on the FE — the wrapper
    // clears the bearer cache, which fires the invalidation event the hook
    // subscribed to at mount. No sessionToken change, no rerender.
    clearCoreRpcTokenCache();

    await waitFor(() => expect(MockEventSource.instances).toHaveLength(2));
    expect(first.close).toHaveBeenCalledTimes(1);
    expect(MockEventSource.instances[1].url).toBe(
      'http://localhost:7788/events/webhooks?token=rpc-token-2'
    );
  });
});
