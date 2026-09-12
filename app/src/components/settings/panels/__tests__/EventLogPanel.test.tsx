import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import EventLogPanel from '../EventLogPanel';

vi.mock('../../../../services/coreRpcClient', () => ({
  getCoreHttpBaseUrl: vi.fn().mockResolvedValue('http://localhost:9999'),
  getCoreRpcToken: vi.fn().mockResolvedValue('test-token'),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

function mockFetchSSE(events: Array<{ domain: string; event: string; detail?: string }>) {
  const lines = events.map(e => `data:${JSON.stringify({ ...e, timestamp: '12:00:00' })}\n`);
  const body = lines.join('');
  const encoder = new TextEncoder();
  const stream = new ReadableStream({
    start(controller) {
      controller.enqueue(encoder.encode(body));
      controller.close();
    },
  });
  global.fetch = vi.fn().mockResolvedValue({ ok: true, body: stream });
}

describe('EventLogPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the panel with header and filter controls', () => {
    global.fetch = vi.fn().mockResolvedValue({ ok: false, body: null });
    renderWithProviders(<EventLogPanel />);
    expect(screen.getByTestId('event-log-panel')).toBeTruthy();
    expect(screen.getByText('settings.developerMenu.eventLog.allTypes')).toBeTruthy();
  });

  it('shows waiting message when connected but no events', async () => {
    const stream = new ReadableStream({
      start() {
        // never enqueue — stays open
      },
    });
    global.fetch = vi.fn().mockResolvedValue({ ok: true, body: stream });
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => {
      expect(screen.getByText('settings.developerMenu.eventLog.waiting')).toBeTruthy();
    });
  });

  it('renders events from SSE stream with domain badges', async () => {
    mockFetchSSE([
      { domain: 'tool', event: 'ToolExecuted' },
      { domain: 'agent', event: 'AgentStarted' },
    ]);
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => {
      expect(screen.getByText('ToolExecuted')).toBeTruthy();
      expect(screen.getByText('AgentStarted')).toBeTruthy();
    });

    expect(screen.getByText('settings.developerMenu.eventLog.badge.tool')).toBeTruthy();
    expect(screen.getByText('settings.developerMenu.eventLog.badge.agent')).toBeTruthy();
  });

  it('renders the backend detail line for events that carry one', async () => {
    // The envelope carries the variant name only, so an MCP transport drop
    // reaches the log with its reason in `detail` (#5931).
    mockFetchSSE([
      {
        domain: 'mcp_client',
        event: 'McpServerTransportDropped',
        detail: 'session ended: broken after 1961ms — connection reset',
      },
      { domain: 'tool', event: 'ToolExecuted' },
    ]);
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => {
      expect(screen.getByText('McpServerTransportDropped')).toBeTruthy();
    });
    expect(screen.getByText('session ended: broken after 1961ms — connection reset')).toBeTruthy();
    // An event without a detail renders exactly as it did before.
    expect(screen.getByText('ToolExecuted')).toBeTruthy();
  });

  it('matches the filter text against the detail line', async () => {
    mockFetchSSE([
      { domain: 'mcp_client', event: 'McpServerTransportDropped', detail: 'connection reset' },
      { domain: 'tool', event: 'ToolExecuted' },
    ]);
    const { container } = renderWithProviders(<EventLogPanel />);

    await waitFor(() => {
      expect(screen.getByText('ToolExecuted')).toBeTruthy();
    });

    const filter = container.querySelector('input')!;
    fireEvent.change(filter, { target: { value: 'connection reset' } });

    await waitFor(() => {
      expect(screen.queryByText('ToolExecuted')).toBeNull();
    });
    expect(screen.getByText('McpServerTransportDropped')).toBeTruthy();
  });

  it('shows disconnected state when fetch fails', async () => {
    global.fetch = vi.fn().mockRejectedValue(new Error('network'));
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => {
      expect(screen.getByText('settings.developerMenu.eventLog.disconnected')).toBeTruthy();
    });
  });

  it('filters events by domain type', async () => {
    mockFetchSSE([
      { domain: 'tool', event: 'ToolA' },
      { domain: 'agent', event: 'AgentB' },
    ]);
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => {
      expect(screen.getByText('ToolA')).toBeTruthy();
    });

    // By label, not by position: the toolbar has more than one select since
    // the workspace-scope control landed (#5966), and `querySelector('select')`
    // silently picks whichever comes first in the DOM.
    const select = screen.getByLabelText(
      'settings.developerMenu.eventLog.allTypes'
    ) as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'tool' } });

    await waitFor(() => {
      expect(screen.queryByText('AgentB')).toBeNull();
    });
    expect(screen.getByText('ToolA')).toBeTruthy();
  });

  it('shows not connected when token is missing', async () => {
    const { getCoreRpcToken } = await import('../../../../services/coreRpcClient');
    vi.mocked(getCoreRpcToken).mockResolvedValueOnce(null as unknown as string);
    global.fetch = vi.fn();
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => {
      expect(screen.getByText('settings.developerMenu.eventLog.notConnected')).toBeTruthy();
    });
    expect(global.fetch).not.toHaveBeenCalled();
  });

  it('exports filtered events as ndjson', async () => {
    mockFetchSSE([{ domain: 'tool', event: 'ToolExport' }]);
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => {
      expect(screen.getByText('ToolExport')).toBeTruthy();
    });

    const createObjectURL = vi.fn().mockReturnValue('blob:test');
    const revokeObjectURL = vi.fn();
    global.URL.createObjectURL = createObjectURL;
    global.URL.revokeObjectURL = revokeObjectURL;

    const downloadBtn = screen.getByText('settings.developerMenu.eventLog.download');
    fireEvent.click(downloadBtn);

    expect(createObjectURL).toHaveBeenCalled();
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:test');
  });

  it('filters events by text input', async () => {
    mockFetchSSE([
      { domain: 'tool', event: 'ToolMatch' },
      { domain: 'tool', event: 'Other' },
    ]);
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => {
      expect(screen.getByText('ToolMatch')).toBeTruthy();
    });

    const input = screen.getByPlaceholderText('settings.developerMenu.eventLog.filterAgent');
    fireEvent.change(input, { target: { value: 'Match' } });

    await waitFor(() => {
      expect(screen.queryByText('Other')).toBeNull();
    });
  });

  it('renders unknown domain as uppercase text', async () => {
    mockFetchSSE([{ domain: 'custom_thing', event: 'Evt' }]);
    renderWithProviders(<EventLogPanel />);

    await waitFor(() => {
      expect(screen.getByText('CUSTOM_THING')).toBeTruthy();
    });
  });

  it('handles scroll and shows jump-to-latest button', async () => {
    mockFetchSSE([{ domain: 'tool', event: 'ScrollTest' }]);
    const { container } = renderWithProviders(<EventLogPanel />);

    await waitFor(() => {
      expect(screen.getByText('ScrollTest')).toBeTruthy();
    });

    // Was `container.querySelector('.max-h-[60vh]')` — the scroll region was
    // addressed by a utility class, so restyling it broke the test without any
    // behaviour changing. A testid is the stable handle.
    const scrollDiv = container.querySelector('[data-testid="event-log-scroll"]')!;
    Object.defineProperty(scrollDiv, 'scrollTop', { value: 100, writable: true });
    fireEvent.scroll(scrollDiv);

    await waitFor(() => {
      expect(screen.getByText('settings.developerMenu.eventLog.jumpToLatest')).toBeTruthy();
    });

    fireEvent.click(screen.getByText('settings.developerMenu.eventLog.jumpToLatest'));
  });
});
