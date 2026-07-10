import { renderHook, waitFor } from '@testing-library/react';
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
