import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockListToolkits = vi.fn();
const mockListConnections = vi.fn();
const mockListAgentReadyToolkits = vi.fn();
const mockOpenhumanComposioGetMode = vi.fn();
let sessionToken = 'jwt-abc';

vi.mock('./composioApi', () => ({
  COMPOSIO_FETCH_TIMEOUT_MS: 8_000,
  listToolkits: (options?: { timeoutMs?: number }) => mockListToolkits(options),
  listConnections: (options?: { timeoutMs?: number }) => mockListConnections(options),
  listAgentReadyToolkits: () => mockListAgentReadyToolkits(),
}));

vi.mock('../coreState/store', async () => {
  const actual = await vi.importActual<typeof import('../coreState/store')>('../coreState/store');
  return { ...actual, getCoreStateSnapshot: () => ({ snapshot: { sessionToken } }) };
});

vi.mock('../../utils/tauriCommands', async () => {
  const actual = await vi.importActual<typeof import('../../utils/tauriCommands')>(
    '../../utils/tauriCommands'
  );
  return { ...actual, openhumanComposioGetMode: () => mockOpenhumanComposioGetMode() };
});

describe('useComposioIntegrations', () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
    // The toolkit catalog is now cached in localStorage (24h TTL); clear it
    // so each test exercises the mocked fetch instead of a prior test's cache.
    window.localStorage.clear();
    sessionToken = 'jwt-abc';
    mockOpenhumanComposioGetMode.mockResolvedValue({
      result: { mode: 'backend', api_key_set: true },
      logs: [],
    });
  });

  it('keeps toolkit cards visible when connections fetch fails', async () => {
    const { useComposioIntegrations } = await import('./hooks');

    mockListToolkits.mockResolvedValue({ toolkits: ['gmail', 'github', 'notion'] });
    mockListConnections.mockRejectedValue(new Error('backend connection listing failed'));

    const { result } = renderHook(() => useComposioIntegrations(0));

    // The connections leg retries silently (#4290) before surfacing, so allow
    // for the retry backoff window before asserting the final error state.
    await waitFor(
      () => {
        expect(result.current.loading).toBe(false);
      },
      { timeout: 3000 }
    );

    expect(result.current.toolkits).toEqual(['gmail', 'github', 'notion']);
    expect(result.current.connectionByToolkit.size).toBe(0);
    expect(result.current.connectionsByToolkit.size).toBe(0);
    expect(result.current.error).toBe('backend connection listing failed');
  });

  it('retries a failed leg silently and clears the error when the retry succeeds (#4290)', async () => {
    const { useComposioIntegrations } = await import('./hooks');

    // Cold-start: first toolkit fetch times out, the silent retry succeeds.
    mockListToolkits
      .mockRejectedValueOnce(new Error('Core RPC openhuman.composio_list_toolkits timed out'))
      .mockResolvedValue({ toolkits: ['gmail'] });
    mockListConnections.mockResolvedValue({ connections: [] });

    const { result } = renderHook(() => useComposioIntegrations(0));

    await waitFor(
      () => {
        expect(result.current.loading).toBe(false);
      },
      { timeout: 3000 }
    );

    // No banner — the transient cold-start timeout self-healed.
    expect(result.current.error).toBeNull();
    expect(result.current.toolkits).toEqual(['gmail']);
    // Exactly one retry (2 attempts total) on the failing leg.
    expect(mockListToolkits).toHaveBeenCalledTimes(2);
  });

  it('surfaces the error only after the silent retry is also exhausted (#4290)', async () => {
    const { useComposioIntegrations } = await import('./hooks');

    mockListToolkits.mockRejectedValue(new Error('backend down'));
    mockListConnections.mockResolvedValue({ connections: [] });

    const { result } = renderHook(() => useComposioIntegrations(0));

    await waitFor(
      () => {
        expect(result.current.loading).toBe(false);
      },
      { timeout: 3000 }
    );

    // Genuine outage is NOT masked — the banner still appears, just later.
    expect(result.current.error).toBe('backend down');
    expect(mockListToolkits).toHaveBeenCalledTimes(2);
  });

  it('does not retry a leg that succeeds on the first attempt (#4290)', async () => {
    const { useComposioIntegrations } = await import('./hooks');

    mockListToolkits.mockResolvedValue({ toolkits: ['gmail'] });
    mockListConnections.mockResolvedValue({ connections: [] });

    const { result } = renderHook(() => useComposioIntegrations(0));

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.error).toBeNull();
    expect(mockListToolkits).toHaveBeenCalledTimes(1);
    expect(mockListConnections).toHaveBeenCalledTimes(1);
  });

  it('exposes the dynamic catalog keyed by canonical slug', async () => {
    const { useComposioIntegrations } = await import('./hooks');

    mockListToolkits.mockResolvedValue({
      toolkits: ['gmail', 'googlecalendar'],
      catalog: [
        { slug: 'gmail', name: 'Gmail', logo: 'https://x/gmail.png', enabled: true },
        // Alias slug must be canonicalized to googlecalendar.
        { slug: 'google_calendar', name: 'Google Calendar', enabled: true },
      ],
    });
    mockListConnections.mockResolvedValue({ connections: [] });

    const { result } = renderHook(() => useComposioIntegrations(0));

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.catalogByToolkit.get('gmail')?.name).toBe('Gmail');
    expect(result.current.catalogByToolkit.get('gmail')?.logo).toBe('https://x/gmail.png');
    expect(result.current.catalogByToolkit.get('googlecalendar')?.name).toBe('Google Calendar');
  });

  it('leaves the catalog empty when the core omits it (back-compat)', async () => {
    const { useComposioIntegrations } = await import('./hooks');

    mockListToolkits.mockResolvedValue({ toolkits: ['gmail'] });
    mockListConnections.mockResolvedValue({ connections: [] });

    const { result } = renderHook(() => useComposioIntegrations(0));

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.toolkits).toEqual(['gmail']);
    expect(result.current.catalogByToolkit.size).toBe(0);
  });

  it('seeds initial state from the durable connection cache for an instant cold-start paint', async () => {
    // Pre-populate the durable cache as if a prior session had persisted it,
    // under the active user's namespace (the cache is inert without a user id).
    window.localStorage.setItem('OPENHUMAN_ACTIVE_USER_ID', 'test-user');
    window.localStorage.setItem(
      'test-user:composio:connections:v1',
      JSON.stringify({
        fetchedAt: Date.now(),
        connections: [{ id: 'seed', toolkit: 'gmail', status: 'ACTIVE' }],
        toolkits: ['gmail'],
        catalog: [{ slug: 'gmail', name: 'Gmail' }],
      })
    );

    // The live fetch hangs so we can observe the seeded state in isolation.
    mockListToolkits.mockReturnValue(new Promise(() => {}));
    mockListConnections.mockReturnValue(new Promise(() => {}));

    const { useComposioIntegrations } = await import('./hooks');
    const { result } = renderHook(() => useComposioIntegrations(0));

    // No loading skeleton — the cached snapshot paints immediately.
    expect(result.current.loading).toBe(false);
    expect(result.current.connectionByToolkit.get('gmail')?.status).toBe('ACTIVE');
    expect(result.current.toolkits).toEqual(['gmail']);
    expect(result.current.catalogByToolkit.get('gmail')?.name).toBe('Gmail');
  });

  it('reconciles a stale cached connection away once the live fetch lands', async () => {
    window.localStorage.setItem('OPENHUMAN_ACTIVE_USER_ID', 'test-user');
    window.localStorage.setItem(
      'test-user:composio:connections:v1',
      JSON.stringify({
        fetchedAt: Date.now(),
        connections: [{ id: 'seed', toolkit: 'gmail', status: 'ACTIVE' }],
        toolkits: ['gmail'],
        catalog: [],
      })
    );
    // Backend now reports the toolkit disconnected.
    mockListToolkits.mockResolvedValue({ toolkits: ['gmail'] });
    mockListConnections.mockResolvedValue({ connections: [] });

    const { useComposioIntegrations } = await import('./hooks');
    const { result } = renderHook(() => useComposioIntegrations(0));

    // Seeded immediately from the cache…
    expect(result.current.connectionByToolkit.get('gmail')?.status).toBe('ACTIVE');
    // …then the live fetch reconciles the stale connection away.
    await waitFor(() => {
      expect(result.current.connectionByToolkit.size).toBe(0);
    });
  });

  it('groups connections by toolkit, sorts by status then createdAt', async () => {
    const { useComposioIntegrations } = await import('./hooks');

    mockListToolkits.mockResolvedValue({ toolkits: ['gmail'] });
    mockListConnections.mockResolvedValue({
      connections: [
        { id: 'c1', toolkit: 'gmail', status: 'EXPIRED', createdAt: '2025-01-01' },
        { id: 'c2', toolkit: 'gmail', status: 'ACTIVE', createdAt: '2025-06-01' },
        { id: 'c3', toolkit: 'gmail', status: 'ACTIVE', createdAt: '2025-03-01' },
        { id: 'c4', toolkit: 'gmail', status: 'PENDING', createdAt: '2025-02-01' },
      ],
    });

    const { result } = renderHook(() => useComposioIntegrations(0));

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    const gmailConns = result.current.connectionsByToolkit.get('gmail');
    expect(gmailConns).toHaveLength(4);
    expect(gmailConns![0].id).toBe('c3');
    expect(gmailConns![1].id).toBe('c2');
    expect(gmailConns![2].id).toBe('c4');
    expect(gmailConns![3].id).toBe('c1');

    expect(result.current.connectionByToolkit.get('gmail')?.id).toBe('c2');
  });

  it('surfaces toolkit fetch errors instead of hiding the UI (composio is always enabled)', async () => {
    const { useComposioIntegrations } = await import('./hooks');

    mockListToolkits.mockRejectedValue(new Error('backend unreachable'));
    mockListConnections.mockRejectedValue(new Error('backend unreachable'));

    const { result } = renderHook(() => useComposioIntegrations(0));

    // Both legs retry silently (#4290) before surfacing — allow the backoff.
    await waitFor(
      () => {
        expect(result.current.loading).toBe(false);
      },
      { timeout: 3000 }
    );

    expect(result.current.toolkits).toEqual([]);
    expect(result.current.connectionByToolkit.size).toBe(0);
    expect(result.current.error).toBe('backend unreachable');
  });

  it('pauses Composio polling on 401 until a credential configuration change', async () => {
    const { useComposioIntegrations } = await import('./hooks');
    mockListToolkits.mockResolvedValue({ toolkits: ['gmail'] });
    mockListConnections.mockResolvedValue({ connections: [] });
    let tick: (() => void) | undefined;
    // Keep Testing Library's waitFor interval alive while intercepting only
    // the hook's own 12_345ms poll. Swallowing all setInterval calls makes
    // waitFor hang indefinitely even after the hook has settled.
    const realSetInterval = window.setInterval.bind(window);
    const timer = vi.spyOn(window, 'setInterval').mockImplementation((handler, interval) => {
      if (interval === 12_345 && typeof handler === 'function') {
        tick = handler;
        return 1234 as unknown as ReturnType<typeof window.setInterval>;
      }
      return realSetInterval(handler, interval) as unknown as ReturnType<typeof window.setInterval>;
    });
    const { result, unmount } = renderHook(() => useComposioIntegrations(12_345));
    try {
      await waitFor(() => expect(result.current.loading).toBe(false));
      expect(tick).toBeDefined();

      // This is a completed authenticated fetch, not an unavailable local key.
      mockListConnections.mockRejectedValueOnce(new Error('HTTP 401 invalid API key'));
      await act(async () => {
        tick?.();
        await Promise.resolve();
        await Promise.resolve();
      });
      const afterUnauthorized = mockListConnections.mock.calls.length;

      // Calling the installed interval callback again must not keep hammering
      // the invalid credentials endpoint, or poll the old tenant repeatedly.
      await act(async () => {
        tick?.();
        await Promise.resolve();
      });
      expect(mockListConnections).toHaveBeenCalledTimes(afterUnauthorized);

      // Once a new key is saved (or the mode changes), normal refresh and
      // periodic polling must resume without requiring component remount.
      await act(async () => {
        window.dispatchEvent(new Event('composio:config-changed'));
        await Promise.resolve();
      });
      await waitFor(() =>
        expect(mockListConnections.mock.calls.length).toBeGreaterThan(afterUnauthorized)
      );
      const afterRefresh = mockListConnections.mock.calls.length;
      await act(async () => {
        tick?.();
        await Promise.resolve();
      });
      expect(mockListConnections).toHaveBeenCalledTimes(afterRefresh + 1);
    } finally {
      unmount();
      timer.mockRestore();
    }
  });

  it('keeps polling on transient Composio failures other than authentication', async () => {
    const { useComposioIntegrations } = await import('./hooks');
    mockListToolkits.mockResolvedValue({ toolkits: [] });
    mockListConnections.mockResolvedValue({ connections: [] });
    let tick: (() => void) | undefined;
    // Keep Testing Library's waitFor interval alive while intercepting only
    // the hook's own 12_345ms poll. Swallowing all setInterval calls makes
    // waitFor hang indefinitely even after the hook has settled.
    const realSetInterval = window.setInterval.bind(window);
    const timer = vi.spyOn(window, 'setInterval').mockImplementation((handler, interval) => {
      if (interval === 12_345 && typeof handler === 'function') {
        tick = handler;
        return 1234 as unknown as ReturnType<typeof window.setInterval>;
      }
      return realSetInterval(handler, interval) as unknown as ReturnType<typeof window.setInterval>;
    });
    const { result, unmount } = renderHook(() => useComposioIntegrations(12_345));
    try {
      await waitFor(() => expect(result.current.loading).toBe(false));
      mockListConnections.mockRejectedValueOnce(new Error('503 service unavailable'));
      await act(async () => {
        tick?.();
        await Promise.resolve();
        await Promise.resolve();
      });
      const first = mockListConnections.mock.calls.length;
      await act(async () => {
        tick?.();
        await Promise.resolve();
      });
      expect(mockListConnections).toHaveBeenCalledTimes(first + 1);
    } finally {
      unmount();
      timer.mockRestore();
    }
  });

  it('keeps polling when the local core bearer is rejected with a 401', async () => {
    const { useComposioIntegrations } = await import('./hooks');
    const { CoreRpcError } = await import('../../services/coreRpcClient');
    mockListToolkits.mockResolvedValue({ toolkits: ['gmail'] });
    mockListConnections.mockResolvedValue({ connections: [] });
    let tick: (() => void) | undefined;
    const realSetInterval = window.setInterval.bind(window);
    const timer = vi.spyOn(window, 'setInterval').mockImplementation((handler, interval) => {
      if (interval === 12_345 && typeof handler === 'function') {
        tick = handler;
        return 1234 as unknown as ReturnType<typeof window.setInterval>;
      }
      return realSetInterval(handler, interval) as unknown as ReturnType<typeof window.setInterval>;
    });
    const { result, unmount } = renderHook(() => useComposioIntegrations(12_345));
    try {
      await waitFor(() => expect(result.current.loading).toBe(false));
      mockListConnections.mockRejectedValueOnce(
        new CoreRpcError('Core RPC HTTP 401: Unauthorized', 'core_auth', 401)
      );
      await act(async () => {
        tick?.();
        await Promise.resolve();
        await Promise.resolve();
      });
      const afterCoreAuth = mockListConnections.mock.calls.length;
      await act(async () => {
        tick?.();
        await Promise.resolve();
      });
      expect(mockListConnections).toHaveBeenCalledTimes(afterCoreAuth + 1);
    } finally {
      unmount();
      timer.mockRestore();
    }
  });

  it('does not supersede a slow successful poll with the next interval tick', async () => {
    const { useComposioIntegrations } = await import('./hooks');
    mockListToolkits.mockResolvedValue({ toolkits: ['gmail'] });
    mockListConnections.mockResolvedValue({ connections: [] });
    let tick: (() => void) | undefined;
    const realSetInterval = window.setInterval.bind(window);
    const timer = vi.spyOn(window, 'setInterval').mockImplementation((handler, interval) => {
      if (interval === 12_345 && typeof handler === 'function') {
        tick = handler;
        return 1234 as unknown as ReturnType<typeof window.setInterval>;
      }
      return realSetInterval(handler, interval) as unknown as ReturnType<typeof window.setInterval>;
    });
    const { result, unmount } = renderHook(() => useComposioIntegrations(12_345));
    try {
      await waitFor(() => expect(result.current.loading).toBe(false));
      let resolveSlow!: (value: {
        connections: { id: string; toolkit: string; status: string }[];
      }) => void;
      mockListConnections.mockImplementationOnce(
        () =>
          new Promise(resolve => {
            resolveSlow = resolve;
          })
      );
      await act(async () => {
        tick?.();
        await Promise.resolve();
      });
      const started = mockListConnections.mock.calls.length;
      await act(async () => {
        tick?.();
        await Promise.resolve();
      });
      expect(mockListConnections).toHaveBeenCalledTimes(started);
      await act(async () => {
        resolveSlow({ connections: [{ id: 'slow', toolkit: 'gmail', status: 'ACTIVE' }] });
        await Promise.resolve();
      });
      expect(result.current.connectionByToolkit.get('gmail')?.id).toBe('slow');
      await act(async () => {
        tick?.();
        await Promise.resolve();
      });
      expect(mockListConnections).toHaveBeenCalledTimes(started + 1);
    } finally {
      unmount();
      timer.mockRestore();
    }
  });

  it('does not let an earlier poll overwrite a successful manual refresh', async () => {
    const { useComposioIntegrations } = await import('./hooks');
    mockListToolkits.mockResolvedValue({ toolkits: ['gmail'] });
    mockListConnections.mockResolvedValue({ connections: [] });
    let tick: (() => void) | undefined;
    const realSetInterval = window.setInterval.bind(window);
    const timer = vi.spyOn(window, 'setInterval').mockImplementation((handler, interval) => {
      if (interval === 12_345 && typeof handler === 'function') {
        tick = handler;
        return 1234 as unknown as ReturnType<typeof window.setInterval>;
      }
      return realSetInterval(handler, interval) as unknown as ReturnType<typeof window.setInterval>;
    });
    const { result, unmount } = renderHook(() => useComposioIntegrations(12_345));
    try {
      await waitFor(() => expect(result.current.loading).toBe(false));
      let resolveOld!: (value: {
        connections: { id: string; toolkit: string; status: string }[];
      }) => void;
      mockListConnections.mockImplementationOnce(
        () =>
          new Promise(resolve => {
            resolveOld = resolve;
          })
      );
      await act(async () => {
        tick?.();
        await Promise.resolve();
      });
      mockListConnections.mockResolvedValueOnce({
        connections: [{ id: 'fresh', toolkit: 'gmail', status: 'ACTIVE' }],
      });
      await act(async () => {
        await result.current.refresh();
      });
      expect(result.current.connectionByToolkit.get('gmail')?.id).toBe('fresh');

      await act(async () => {
        resolveOld({ connections: [{ id: 'stale', toolkit: 'gmail', status: 'ACTIVE' }] });
        await Promise.resolve();
      });
      expect(result.current.connectionByToolkit.get('gmail')?.id).toBe('fresh');
    } finally {
      unmount();
      timer.mockRestore();
    }
  });

  it('ignores an older unauthorized poll after a successful manual refresh', async () => {
    const { useComposioIntegrations } = await import('./hooks');
    mockListToolkits.mockResolvedValue({ toolkits: ['gmail'] });
    mockListConnections.mockResolvedValue({ connections: [] });
    let tick: (() => void) | undefined;
    const realSetInterval = window.setInterval.bind(window);
    const timer = vi.spyOn(window, 'setInterval').mockImplementation((handler, interval) => {
      if (interval === 12_345 && typeof handler === 'function') {
        tick = handler;
        return 1234 as unknown as ReturnType<typeof window.setInterval>;
      }
      return realSetInterval(handler, interval) as unknown as ReturnType<typeof window.setInterval>;
    });
    const { result, unmount } = renderHook(() => useComposioIntegrations(12_345));
    try {
      await waitFor(() => expect(result.current.loading).toBe(false));
      let rejectOld!: (error: Error) => void;
      mockListConnections.mockImplementationOnce(
        () =>
          new Promise((_resolve, reject) => {
            rejectOld = reject;
          })
      );
      await act(async () => {
        tick?.();
        await Promise.resolve();
      });
      mockListConnections.mockResolvedValueOnce({
        connections: [{ id: 'fresh', toolkit: 'gmail', status: 'ACTIVE' }],
      });
      await act(async () => {
        await result.current.refresh();
      });
      expect(result.current.connectionByToolkit.get('gmail')?.id).toBe('fresh');

      await act(async () => {
        rejectOld(new Error('HTTP 401 invalid API key'));
        await Promise.resolve();
        await Promise.resolve();
      });
      expect(result.current.connectionByToolkit.get('gmail')?.id).toBe('fresh');
      const afterRejectedOldPoll = mockListConnections.mock.calls.length;
      await act(async () => {
        tick?.();
        await Promise.resolve();
      });
      expect(mockListConnections).toHaveBeenCalledTimes(afterRejectedOldPoll + 1);
    } finally {
      unmount();
      timer.mockRestore();
    }
  });

  it('resumes periodic polling after a successful manual refresh of rejected credentials', async () => {
    const { useComposioIntegrations } = await import('./hooks');
    mockListToolkits.mockResolvedValue({ toolkits: ['gmail'] });
    mockListConnections.mockResolvedValue({ connections: [] });
    let tick: (() => void) | undefined;
    const realSetInterval = window.setInterval.bind(window);
    const timer = vi.spyOn(window, 'setInterval').mockImplementation((handler, interval) => {
      if (interval === 12_345 && typeof handler === 'function') {
        tick = handler;
        return 1234 as unknown as ReturnType<typeof window.setInterval>;
      }
      return realSetInterval(handler, interval) as unknown as ReturnType<typeof window.setInterval>;
    });
    const { result, unmount } = renderHook(() => useComposioIntegrations(12_345));
    try {
      await waitFor(() => expect(result.current.loading).toBe(false));
      mockListConnections.mockRejectedValueOnce(new Error('HTTP 401 invalid API key'));
      await act(async () => {
        tick?.();
        await Promise.resolve();
        await Promise.resolve();
      });
      const paused = mockListConnections.mock.calls.length;
      await act(async () => {
        tick?.();
        await Promise.resolve();
      });
      expect(mockListConnections).toHaveBeenCalledTimes(paused);
      await act(async () => {
        await result.current.refresh();
      });
      const refreshed = mockListConnections.mock.calls.length;
      await act(async () => {
        tick?.();
        await Promise.resolve();
      });
      expect(mockListConnections).toHaveBeenCalledTimes(refreshed + 1);
    } finally {
      unmount();
      timer.mockRestore();
    }
  });

  it('serializes poll requests and pauses on rejection until identity recovery', async () => {
    const { useComposioIntegrations } = await import('./hooks');
    mockListToolkits.mockResolvedValue({ toolkits: ['gmail'] });
    mockListConnections.mockResolvedValue({ connections: [] });
    let tick: (() => void) | undefined;
    const realSetInterval = window.setInterval.bind(window);
    const timer = vi.spyOn(window, 'setInterval').mockImplementation((handler, interval) => {
      if (interval === 12_345 && typeof handler === 'function') {
        tick = handler;
        return 1234 as unknown as ReturnType<typeof window.setInterval>;
      }
      return realSetInterval(handler, interval) as unknown as ReturnType<typeof window.setInterval>;
    });
    const { result, unmount } = renderHook(() => useComposioIntegrations(12_345));
    try {
      await waitFor(() => expect(result.current.loading).toBe(false));
      let rejectOldPoll: ((error: Error) => void) | undefined;
      mockListConnections.mockImplementationOnce(
        () =>
          new Promise((_resolve, reject) => {
            rejectOldPoll = reject;
          })
      );
      await act(async () => {
        tick?.();
        await Promise.resolve();
      });
      const firstInFlight = mockListConnections.mock.calls.length;
      await act(async () => {
        tick?.();
        await Promise.resolve();
      });
      expect(mockListConnections).toHaveBeenCalledTimes(firstInFlight);
      await act(async () => {
        rejectOldPoll?.(new Error('HTTP 401 invalid API key'));
        await Promise.resolve();
        await Promise.resolve();
      });
      await act(async () => {
        tick?.();
        await Promise.resolve();
      });
      expect(mockListConnections).toHaveBeenCalledTimes(firstInFlight);
      await act(async () => {
        window.dispatchEvent(new Event('composio:config-changed'));
        await Promise.resolve();
      });
      await waitFor(() => {
        expect(mockListConnections.mock.calls.length).toBeGreaterThan(firstInFlight);
      });
      const afterRecovery = mockListConnections.mock.calls.length;
      await act(async () => {
        tick?.();
        await Promise.resolve();
      });
      expect(mockListConnections).toHaveBeenCalledTimes(afterRecovery + 1);
    } finally {
      unmount();
      timer.mockRestore();
    }
  });

  it('skips toolkit fetch and polling for local sessions without a composio api key', async () => {
    sessionToken = 'header.payload.local';
    mockOpenhumanComposioGetMode.mockResolvedValue({
      result: { mode: 'direct', api_key_set: false },
      logs: [],
    });

    const { useComposioIntegrations } = await import('./hooks');
    const { result } = renderHook(() => useComposioIntegrations(10));

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.toolkits).toEqual([]);
    expect(result.current.connectionByToolkit.size).toBe(0);
    expect(result.current.error).toBeNull();
    expect(mockListToolkits).not.toHaveBeenCalled();
    expect(mockListConnections).not.toHaveBeenCalled();
  });
});

describe('useAgentReadyComposioToolkits', () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
  });

  it('returns a normalized Set of agent-ready toolkit slugs on success', async () => {
    const { useAgentReadyComposioToolkits } = await import('./hooks');

    mockListAgentReadyToolkits.mockResolvedValue({
      toolkits: ['gmail', 'one_drive', 'EXCEL', 'todoist'],
    });

    const { result } = renderHook(() => useAgentReadyComposioToolkits());

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    // canonicalizeComposioToolkitSlug normalizes case and aliases.
    expect(result.current.agentReady.has('gmail')).toBe(true);
    expect(result.current.agentReady.has('one_drive')).toBe(true);
    expect(result.current.agentReady.has('excel')).toBe(true);
    expect(result.current.agentReady.has('todoist')).toBe(true);
    // Uncatalogued toolkit must NOT appear — the UI relies on this
    // to drive the preview-badge logic (issue #2283).
    expect(result.current.agentReady.has('clickup')).toBe(false);
    expect(result.current.error).toBeNull();
  });

  it('returns an empty set and surfaces error when the RPC fails', async () => {
    const { useAgentReadyComposioToolkits } = await import('./hooks');

    mockListAgentReadyToolkits.mockRejectedValue(new Error('rpc unavailable'));

    const { result } = renderHook(() => useAgentReadyComposioToolkits());

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    // Failure must NOT label every toolkit as preview — surface the
    // error and let the caller decide how to degrade.
    expect(result.current.agentReady.size).toBe(0);
    expect(result.current.error).toBe('rpc unavailable');
  });

  it('handles a missing toolkits field without throwing', async () => {
    const { useAgentReadyComposioToolkits } = await import('./hooks');

    mockListAgentReadyToolkits.mockResolvedValue({});

    const { result } = renderHook(() => useAgentReadyComposioToolkits());

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.agentReady.size).toBe(0);
    expect(result.current.error).toBeNull();
  });
});
