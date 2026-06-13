/**
 * Coverage for the auth'd SSE wiring added in #1922 — the
 * `WebhooksDebugPanel` mount-once SSE effect that subscribes to
 * `/events/webhooks?token=…` and re-routes deliveries into `setLastEvent`
 * + `loadData`.
 *
 * Heavy provider chain mocked at module boundary; tests assert only the
 * SSE-side observable behaviour (constructor URL, skip-on-null,
 * webhooks_debug event handling).
 *
 * Additional tests target uncovered changed lines (diff-cover report):
 * 197,221,225-226,231,239,253,257-258,261,263,272,277,283,297,300,304,322,330,332
 *
 * These cover:
 * - Registrations section: list rendering of each registration row
 * - Logs section: list rendering, log selection, log detail view (PayloadBlock)
 * - Last-event display
 * - Clear logs button interactions
 * - Error states
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Recording EventSource stub — jsdom has no native impl.
class MockEventSource {
  static instances: MockEventSource[] = [];
  url: string;
  onerror: (() => void) | null = null;
  // The component registers one listener for 'webhooks_debug'; capture it
  // so a test can replay an event and exercise the body of the callback.
  listeners = new Map<string, (event: MessageEvent<string>) => void>();
  close = vi.fn();
  addEventListener = vi.fn((type: string, handler: (event: MessageEvent<string>) => void) => {
    this.listeners.set(type, handler);
  });
  constructor(url: string) {
    this.url = url;
    MockEventSource.instances.push(this);
  }
  fire(type: string, data: unknown) {
    const handler = this.listeners.get(type);
    if (!handler) throw new Error(`no handler for ${type}`);
    handler(new MessageEvent(type, { data: JSON.stringify(data) }));
  }
}

const { mockGetCoreRpcToken, mockGetCoreHttpBaseUrl, mockListLogs, mockListRegs, mockClearLogs } =
  vi.hoisted(() => ({
    mockGetCoreRpcToken: vi.fn<() => Promise<string | null>>(),
    mockGetCoreHttpBaseUrl: vi.fn<() => Promise<string>>(),
    mockListLogs: vi.fn(),
    mockListRegs: vi.fn(),
    mockClearLogs: vi.fn(),
  }));

vi.mock('../../../../services/coreRpcClient', async () => {
  const actual = await vi.importActual<typeof import('../../../../services/coreRpcClient')>(
    '../../../../services/coreRpcClient'
  );
  return {
    ...actual,
    getCoreRpcToken: mockGetCoreRpcToken,
    getCoreHttpBaseUrl: mockGetCoreHttpBaseUrl,
  };
});

vi.mock('../../../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (key: string) => key, locale: 'en', setLocale: vi.fn() }),
}));

vi.mock('../../../../hooks/useBackendUrl', () => ({ useBackendUrl: () => 'http://mock-backend' }));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

vi.mock('../../../../services/api/tunnelsApi', () => ({
  tunnelsApi: {
    getTunnels: vi.fn().mockResolvedValue([]),
    ingressUrl: (_base: string, uuid: string) => `http://mock-backend/webhook/${uuid}`,
  },
}));

vi.mock('../../../../utils/tauriCommands', () => ({
  openhumanWebhooksClearLogs: (...args: unknown[]) => mockClearLogs(...args),
  openhumanWebhooksListLogs: mockListLogs,
  openhumanWebhooksListRegistrations: mockListRegs,
}));

vi.mock('../components/SettingsHeader', () => ({ default: () => null }));

// ------------------------------------------------------------------
// Fixture builders
// ------------------------------------------------------------------

const makeReg = (overrides: Record<string, unknown> = {}) => ({
  tunnel_uuid: 'uuid-abc-123',
  tunnel_name: 'My Webhook',
  target_kind: 'skill',
  skill_id: 'skill-alpha',
  ...overrides,
});

const makeLog = (overrides: Record<string, unknown> = {}) => ({
  correlation_id: 'corr-001',
  method: 'POST',
  path: '/hook/alpha',
  status_code: 200,
  stage: 'delivered',
  skill_id: 'skill-alpha',
  tunnel_name: 'My Webhook',
  updated_at: Date.now() - 5000,
  request_headers: { 'content-type': 'application/json' },
  request_query: {},
  request_body: btoa('{"key":"value"}'),
  response_headers: {},
  response_body: btoa('{"ok":true}'),
  error_message: null,
  raw_payload: null,
  ...overrides,
});

describe('WebhooksDebugPanel — SSE auth wiring (#1922)', () => {
  let originalEventSource: typeof globalThis.EventSource | undefined;

  beforeEach(() => {
    MockEventSource.instances.length = 0;
    originalEventSource = (globalThis as unknown as { EventSource?: typeof globalThis.EventSource })
      .EventSource;
    (globalThis as unknown as { EventSource: typeof MockEventSource }).EventSource =
      MockEventSource;
    mockGetCoreRpcToken.mockReset();
    mockGetCoreHttpBaseUrl.mockReset();
    mockGetCoreHttpBaseUrl.mockResolvedValue('http://localhost:7788');
    mockListLogs.mockReset();
    mockListLogs.mockResolvedValue({ result: { result: { logs: [] } } });
    mockListRegs.mockReset();
    mockListRegs.mockResolvedValue({ result: { result: { registrations: [] } } });
    mockClearLogs.mockReset();
    mockClearLogs.mockResolvedValue({});
  });

  afterEach(() => {
    if (originalEventSource) {
      (globalThis as unknown as { EventSource: typeof globalThis.EventSource }).EventSource =
        originalEventSource;
    } else {
      delete (globalThis as unknown as { EventSource?: typeof MockEventSource }).EventSource;
    }
  });

  it('opens EventSource with ?token=<bearer> when token resolves', async () => {
    mockGetCoreRpcToken.mockResolvedValue('rpc-token-debug-1');
    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');

    render(<WebhooksDebugPanel />);

    expect(screen.getByTestId('webhooks-debug-panel')).toBeInTheDocument();
    await waitFor(() => expect(MockEventSource.instances).toHaveLength(1));
    expect(MockEventSource.instances[0].url).toBe(
      'http://localhost:7788/events/webhooks?token=rpc-token-debug-1'
    );
  });

  it('skips EventSource when no token is available', async () => {
    mockGetCoreRpcToken.mockResolvedValue(null);
    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');

    render(<WebhooksDebugPanel />);

    await waitFor(() => {
      expect(mockGetCoreRpcToken).toHaveBeenCalled();
      expect(mockGetCoreHttpBaseUrl).toHaveBeenCalled();
      expect(MockEventSource.instances).toHaveLength(0);
    });
  });

  it('reloads logs + registrations when a webhooks_debug event fires', async () => {
    mockGetCoreRpcToken.mockResolvedValue('rpc-token-debug-2');
    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');

    render(<WebhooksDebugPanel />);

    await waitFor(() => expect(MockEventSource.instances).toHaveLength(1));
    const es = MockEventSource.instances[0];

    mockListLogs.mockClear();
    mockListRegs.mockClear();

    es.fire('webhooks_debug', { event_type: 'log_appended' });

    await waitFor(() => expect(mockListLogs).toHaveBeenCalled());
    expect(mockListRegs).toHaveBeenCalled();
  });
});

describe('WebhooksDebugPanel — rendering & interaction (uncovered lines)', () => {
  let originalEventSource: typeof globalThis.EventSource | undefined;

  beforeEach(() => {
    MockEventSource.instances.length = 0;
    originalEventSource = (globalThis as unknown as { EventSource?: typeof globalThis.EventSource })
      .EventSource;
    (globalThis as unknown as { EventSource: typeof MockEventSource }).EventSource =
      MockEventSource;
    mockGetCoreRpcToken.mockResolvedValue('tok-render');
    mockGetCoreHttpBaseUrl.mockResolvedValue('http://localhost:7788');
    mockListLogs.mockResolvedValue({ result: { result: { logs: [] } } });
    mockListRegs.mockResolvedValue({ result: { result: { registrations: [] } } });
    mockClearLogs.mockResolvedValue({});
  });

  afterEach(() => {
    vi.clearAllMocks();
    if (originalEventSource) {
      (globalThis as unknown as { EventSource: typeof globalThis.EventSource }).EventSource =
        originalEventSource;
    } else {
      delete (globalThis as unknown as { EventSource?: typeof MockEventSource }).EventSource;
    }
  });

  // ── Registrations section (lines 221, 225-226, 231, 239) ──────────────────

  it('renders empty state when no registrations (line 221)', async () => {
    mockListRegs.mockResolvedValue({ result: { result: { registrations: [] } } });
    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');
    render(<WebhooksDebugPanel />);

    await waitFor(() =>
      expect(screen.getByText('webhooks.noActiveRegistrations')).toBeInTheDocument()
    );
  });

  it('renders registration row with name, kind, skill (lines 225-226, 231, 239)', async () => {
    mockListRegs.mockResolvedValue({ result: { result: { registrations: [makeReg()] } } });
    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');
    render(<WebhooksDebugPanel />);

    await waitFor(() => expect(screen.getByText('My Webhook')).toBeInTheDocument());
    expect(screen.getByText('skill')).toBeInTheDocument();
    expect(screen.getByText('skill-alpha')).toBeInTheDocument();
    // Ingress URL rendered from tunnelsApi (line 239)
    expect(screen.getByText('http://mock-backend/webhook/uuid-abc-123')).toBeInTheDocument();
  });

  it('uses tunnel_uuid when tunnel_name is empty (line 231)', async () => {
    mockListRegs.mockResolvedValue({
      result: {
        result: { registrations: [makeReg({ tunnel_name: '', tunnel_uuid: 'uuid-no-name' })] },
      },
    });
    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');
    render(<WebhooksDebugPanel />);

    await waitFor(() => expect(screen.getByText('uuid-no-name')).toBeInTheDocument());
  });

  // ── Logs section (lines 253, 257-258, 261, 263) ───────────────────────────

  it('renders empty logs state when no captured requests (line 253)', async () => {
    mockListLogs.mockResolvedValue({ result: { result: { logs: [] } } });
    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');
    render(<WebhooksDebugPanel />);

    await waitFor(() =>
      expect(screen.getByText('webhooks.noRequestsCaptured')).toBeInTheDocument()
    );
  });

  it('renders log entry rows (lines 257-258, 261, 263)', async () => {
    const log = makeLog();
    mockListLogs.mockResolvedValue({ result: { result: { logs: [log] } } });
    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');
    render(<WebhooksDebugPanel />);

    // 'POST /hook/alpha' appears in both the log list row AND the detail panel
    // (first entry auto-selected), so use getAllByText.
    await waitFor(() =>
      expect(screen.getAllByText('POST /hook/alpha').length).toBeGreaterThanOrEqual(1)
    );
    expect(document.body.textContent).toContain('200');
    expect(document.body.textContent).toContain('My Webhook');
  });

  it('renders unrouted label for entries with no skill_id (line 277)', async () => {
    const log = makeLog({ skill_id: null });
    mockListLogs.mockResolvedValue({ result: { result: { logs: [log] } } });
    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');
    render(<WebhooksDebugPanel />);

    // 'webhooks.unrouted' may appear in both the log row and the detail panel
    await waitFor(() =>
      expect(screen.getAllByText(/webhooks\.unrouted/).length).toBeGreaterThanOrEqual(1)
    );
  });

  // ── Selected log detail view (lines 272, 283, 297, 300, 304) ─────────────

  it('renders detail view for the first log entry by default (line 283)', async () => {
    const log = makeLog();
    mockListLogs.mockResolvedValue({ result: { result: { logs: [log] } } });
    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');
    render(<WebhooksDebugPanel />);

    // Detail block shows method + path, correlation_id, stage badge, skill badge (lines 285-302)
    await waitFor(() =>
      expect(screen.getAllByText('POST /hook/alpha').length).toBeGreaterThanOrEqual(1)
    );
    expect(screen.getByText('corr-001')).toBeInTheDocument();
    expect(screen.getByText('delivered')).toBeInTheDocument();
  });

  it('selects a different log entry on click (line 261)', async () => {
    const log1 = makeLog({ correlation_id: 'corr-001', path: '/hook/first', method: 'GET' });
    const log2 = makeLog({ correlation_id: 'corr-002', path: '/hook/second', method: 'POST' });
    mockListLogs.mockResolvedValue({ result: { result: { logs: [log1, log2] } } });
    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');
    render(<WebhooksDebugPanel />);

    // Both text values may appear in the list row AND the auto-selected detail panel.
    await waitFor(() =>
      expect(screen.getAllByText('GET /hook/first').length).toBeGreaterThanOrEqual(1)
    );

    // Click the second row entry — use getAllByText and click the first matching element
    const secondEntry = screen.getAllByText('POST /hook/second');
    fireEvent.click(secondEntry[0]);
    await waitFor(() => expect(screen.getByText('corr-002')).toBeInTheDocument());
  });

  it('renders raw_payload PayloadBlock when raw_payload is non-null (line 332)', async () => {
    const log = makeLog({ raw_payload: { event: 'push', ref: 'refs/heads/main' } });
    mockListLogs.mockResolvedValue({ result: { result: { logs: [log] } } });
    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');
    render(<WebhooksDebugPanel />);

    await waitFor(() => expect(screen.getByText('webhooks.rawPayload')).toBeInTheDocument());
  });

  it('renders error_message in detail view when present (line 304)', async () => {
    const log = makeLog({ error_message: 'upstream timeout 30s' });
    mockListLogs.mockResolvedValue({ result: { result: { logs: [log] } } });
    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');
    render(<WebhooksDebugPanel />);

    await waitFor(() => expect(screen.getByText('upstream timeout 30s')).toBeInTheDocument());
  });

  it('shows pending in badge when status_code is null (line 300)', async () => {
    const log = makeLog({ status_code: null });
    mockListLogs.mockResolvedValue({ result: { result: { logs: [log] } } });
    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');
    render(<WebhooksDebugPanel />);

    await waitFor(() => expect(screen.getByText('webhooks.pending')).toBeInTheDocument());
  });

  // ── Last event display (line 208-215) ─────────────────────────────────────

  it('shows last event block when SSE fires a webhooks_debug event (line 208)', async () => {
    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');
    render(<WebhooksDebugPanel />);

    await waitFor(() => expect(MockEventSource.instances).toHaveLength(1));
    const es = MockEventSource.instances[0];

    es.fire('webhooks_debug', { event_type: 'log_appended', timestamp: 1700000000000 });

    // 'webhooks.lastEvent' is a text node adjacent to ': log_appended' in the same div —
    // use body.textContent to avoid split-text-node matching issues.
    await waitFor(() => expect(document.body.textContent).toContain('webhooks.lastEvent'));
    expect(document.body.textContent).toContain('log_appended');
  });

  // ── Refresh button (line 197) ──────────────────────────────────────────────

  it('reloads data when refresh button is clicked (line 197)', async () => {
    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');
    render(<WebhooksDebugPanel />);

    await waitFor(() => expect(screen.getByText('webhooks.refresh')).toBeInTheDocument());

    mockListLogs.mockClear();
    mockListRegs.mockClear();

    fireEvent.click(screen.getByText('webhooks.refresh'));

    await waitFor(() => expect(mockListLogs).toHaveBeenCalled());
    expect(mockListRegs).toHaveBeenCalled();
  });

  // ── Clear logs button (line 322, 330) ─────────────────────────────────────

  it('clear logs button is disabled when logs list is empty (line 322)', async () => {
    mockListLogs.mockResolvedValue({ result: { result: { logs: [] } } });
    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');
    render(<WebhooksDebugPanel />);

    await waitFor(() => expect(screen.getByText('webhooks.clearLogs')).toBeInTheDocument());
    const clearBtn = screen.getByText('webhooks.clearLogs').closest('button') as HTMLButtonElement;
    expect(clearBtn.disabled).toBe(true);
  });

  it('clears logs when confirm accepted (line 322, 330)', async () => {
    const log = makeLog();
    mockListLogs.mockResolvedValue({ result: { result: { logs: [log] } } });
    vi.spyOn(window, 'confirm').mockReturnValue(true);

    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');
    render(<WebhooksDebugPanel />);

    await waitFor(() => expect(screen.getByText('webhooks.clearLogs')).toBeInTheDocument());

    const clearBtn = screen.getByText('webhooks.clearLogs').closest('button') as HTMLButtonElement;
    fireEvent.click(clearBtn);

    await waitFor(() => expect(mockClearLogs).toHaveBeenCalled());
  });

  it('skips clearLogs when confirm is cancelled', async () => {
    const log = makeLog();
    mockListLogs.mockResolvedValue({ result: { result: { logs: [log] } } });
    vi.spyOn(window, 'confirm').mockReturnValue(false);

    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');
    render(<WebhooksDebugPanel />);

    await waitFor(() => screen.getByText('webhooks.clearLogs'));
    const clearBtn = screen.getByText('webhooks.clearLogs').closest('button') as HTMLButtonElement;
    fireEvent.click(clearBtn);

    expect(mockClearLogs).not.toHaveBeenCalled();
  });

  it('shows error state when loadData fails (line 332)', async () => {
    mockListLogs.mockRejectedValue(new Error('network error'));
    const { default: WebhooksDebugPanel } = await import('../WebhooksDebugPanel');
    render(<WebhooksDebugPanel />);

    await waitFor(() => expect(screen.getByText(/network error/)).toBeInTheDocument());
  });
});
