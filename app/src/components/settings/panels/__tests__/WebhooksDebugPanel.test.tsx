/**
 * Coverage for the auth'd SSE wiring added in #1922 — the
 * `WebhooksDebugPanel` mount-once SSE effect that subscribes to
 * `/events/webhooks?token=…` and re-routes deliveries into `setLastEvent`
 * + `loadData`.
 *
 * Heavy provider chain mocked at module boundary; tests assert only the
 * SSE-side observable behaviour (constructor URL, skip-on-null,
 * webhooks_debug event handling).
 */
import { render, screen, waitFor } from '@testing-library/react';
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

const { mockGetCoreRpcToken, mockGetCoreHttpBaseUrl, mockListLogs, mockListRegs } = vi.hoisted(
  () => ({
    mockGetCoreRpcToken: vi.fn<() => Promise<string | null>>(),
    mockGetCoreHttpBaseUrl: vi.fn<() => Promise<string>>(),
    mockListLogs: vi.fn(),
    mockListRegs: vi.fn(),
  })
);

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
  tunnelsApi: { getTunnels: vi.fn().mockResolvedValue([]) },
}));

vi.mock('../../../../utils/tauriCommands', () => ({
  openhumanWebhooksClearLogs: vi.fn(),
  openhumanWebhooksListLogs: mockListLogs,
  openhumanWebhooksListRegistrations: mockListRegs,
}));

vi.mock('../components/SettingsHeader', () => ({ default: () => null }));

describe('WebhooksDebugPanel — SSE auth wiring (#1922)', () => {
  // Save the prior global so we restore it instead of unconditionally
  // deleting in teardown — avoids cross-test side effects if another
  // suite/setup installs its own EventSource.
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

    // SSE effect runs Promise.all (both resolvers fire) then bails on the
    // null URL from buildWebhookEventsUrl. Wait on the observable signal —
    // both resolvers having been called — instead of a real-time sleep so
    // the test is deterministic on slow CI.
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

    // Initial mount already calls loadData once. Reset so the assertion
    // below isolates the reload caused by the SSE event.
    mockListLogs.mockClear();
    mockListRegs.mockClear();

    es.fire('webhooks_debug', { event_type: 'log_appended' });

    await waitFor(() => expect(mockListLogs).toHaveBeenCalled());
    expect(mockListRegs).toHaveBeenCalled();
  });
});
