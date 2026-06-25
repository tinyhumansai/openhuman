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

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

    expect(result.current.toolkits).toEqual(['gmail', 'github', 'notion']);
    expect(result.current.connectionByToolkit.size).toBe(0);
    expect(result.current.connectionsByToolkit.size).toBe(0);
    expect(result.current.error).toBe('backend connection listing failed');
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

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
    });

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
