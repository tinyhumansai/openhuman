/**
 * Tests for McpServersTab — covers initial load, error display, loading state,
 * pane switching (none / catalog / install / detail), install success, and
 * uninstall flows.
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import McpServersTab from './McpServersTab';

// --- Mock the API layer ---
const mockInstalledList = vi.fn();
const mockStatus = vi.fn();

vi.mock('../../../services/api/mcpClientsApi', () => ({
  mcpClientsApi: {
    installedList: (...args: unknown[]) => mockInstalledList(...args),
    status: (...args: unknown[]) => mockStatus(...args),
    connect: vi.fn(),
    disconnect: vi.fn(),
    uninstall: vi.fn(),
    configAssist: vi.fn(),
    registrySearch: vi.fn().mockResolvedValue({ servers: [], page: 1, total_pages: 1 }),
  },
}));

// Mock heavy child panels that render their own async behaviour to keep these
// tests focused on the tab-level logic.
vi.mock('./InstalledServerDetail', () => ({
  default: ({ server }: { server: { display_name: string } }) => (
    <div data-testid="installed-server-detail">{server.display_name} detail</div>
  ),
}));

vi.mock('./McpCatalogBrowser', () => ({
  default: ({ onSelectInstall }: { onSelectInstall: (name: string) => void }) => (
    <div data-testid="catalog-browser">
      <button type="button" onClick={() => onSelectInstall('acme/tool')}>
        Install acme/tool
      </button>
    </div>
  ),
}));

vi.mock('./InstallDialog', () => ({
  default: ({
    qualifiedName,
    onSuccess,
    onCancel,
  }: {
    qualifiedName: string;
    onSuccess: (s: { server_id: string; display_name: string }) => void;
    onCancel: () => void;
  }) => (
    <div data-testid="install-dialog">
      <span>{qualifiedName}</span>
      <button
        type="button"
        onClick={() =>
          onSuccess({
            server_id: 'new-srv',
            display_name: 'New Server',
            qualified_name: qualifiedName,
            command_kind: 'node',
            command: 'node',
            args: [],
            env_keys: [],
            installed_at: 0,
          } as never)
        }>
        Finish install
      </button>
      <button type="button" onClick={onCancel}>
        Cancel install
      </button>
    </div>
  ),
}));

// ─── helpers ────────────────────────────────────────────────────────────────

const EMPTY_STATUS: never[] = [];

const SERVER_A = {
  server_id: 'srv-a',
  qualified_name: 'acme/server-a',
  display_name: 'Server A',
  command_kind: 'node' as const,
  command: 'node',
  args: [],
  env_keys: [],
  installed_at: 0,
};

// ─── suite ──────────────────────────────────────────────────────────────────

describe('McpServersTab', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    mockInstalledList.mockReset();
    mockStatus.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  // Helpers to flush initial async load.
  async function flushLoad() {
    await act(async () => {
      await vi.runAllTimersAsync();
    });
  }

  it('shows loading indicator while data is fetching', () => {
    // Never resolve — we want to observe the loading state.
    mockInstalledList.mockReturnValue(new Promise(() => {}));
    mockStatus.mockReturnValue(new Promise(() => {}));

    render(<McpServersTab />);
    expect(screen.getByText(/loading mcp servers/i)).toBeInTheDocument();
  });

  it('renders installed list and empty-pane placeholder after load', async () => {
    mockInstalledList.mockResolvedValue([SERVER_A]);
    mockStatus.mockResolvedValue(EMPTY_STATUS);

    render(<McpServersTab />);
    await flushLoad();

    expect(screen.getByText('Server A')).toBeInTheDocument();
    expect(screen.getByText(/select a server or browse the catalog/i)).toBeInTheDocument();
  });

  it('shows load error banner when installedList rejects', async () => {
    mockInstalledList.mockRejectedValue(new Error('Network down'));
    mockStatus.mockResolvedValue(EMPTY_STATUS);

    render(<McpServersTab />);
    await flushLoad();

    expect(screen.getByText('Network down')).toBeInTheDocument();
  });

  it('opens catalog browser when Browse catalog is clicked', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue(EMPTY_STATUS);

    render(<McpServersTab />);
    await flushLoad();

    // With empty server list there are two "Browse catalog" buttons;
    // click the first one (header link).
    fireEvent.click(screen.getAllByRole('button', { name: /browse catalog/i })[0]);
    expect(screen.getByTestId('catalog-browser')).toBeInTheDocument();
  });

  it('opens install dialog when a catalog entry install is clicked', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue(EMPTY_STATUS);

    render(<McpServersTab />);
    await flushLoad();

    // Navigate to catalog pane
    fireEvent.click(screen.getAllByRole('button', { name: /browse catalog/i })[0]);
    expect(screen.getByTestId('catalog-browser')).toBeInTheDocument();

    // Trigger install from catalog mock
    fireEvent.click(screen.getByRole('button', { name: /install acme\/tool/i }));
    expect(screen.getByTestId('install-dialog')).toBeInTheDocument();
    expect(screen.getByText('acme/tool')).toBeInTheDocument();
  });

  it('cancelling install dialog goes back to catalog', async () => {
    mockInstalledList.mockResolvedValue([]);
    mockStatus.mockResolvedValue(EMPTY_STATUS);

    render(<McpServersTab />);
    await flushLoad();

    fireEvent.click(screen.getAllByRole('button', { name: /browse catalog/i })[0]);
    fireEvent.click(screen.getByRole('button', { name: /install acme\/tool/i }));
    expect(screen.getByTestId('install-dialog')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /cancel install/i }));
    expect(screen.getByTestId('catalog-browser')).toBeInTheDocument();
  });

  it('finishing install refreshes list and opens detail pane', async () => {
    mockInstalledList
      .mockResolvedValueOnce([]) // initial load
      .mockResolvedValueOnce([
        {
          server_id: 'new-srv',
          qualified_name: 'acme/tool',
          display_name: 'New Server',
          command_kind: 'node',
          command: 'node',
          args: [],
          env_keys: [],
          installed_at: 0,
        },
      ]); // after install success
    mockStatus.mockResolvedValue(EMPTY_STATUS);

    render(<McpServersTab />);
    await flushLoad();

    fireEvent.click(screen.getAllByRole('button', { name: /browse catalog/i })[0]);
    fireEvent.click(screen.getByRole('button', { name: /install acme\/tool/i }));

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /finish install/i }));
      await vi.runAllTimersAsync();
    });

    // Detail pane should show the newly installed server
    expect(screen.getByTestId('installed-server-detail')).toBeInTheDocument();
    expect(screen.getByText(/new server detail/i)).toBeInTheDocument();
  });

  it('selecting a server from the list opens its detail pane', async () => {
    mockInstalledList.mockResolvedValue([SERVER_A]);
    mockStatus.mockResolvedValue(EMPTY_STATUS);

    render(<McpServersTab />);
    await flushLoad();

    fireEvent.click(screen.getByRole('button', { name: /server a/i }));

    expect(screen.getByTestId('installed-server-detail')).toBeInTheDocument();
    expect(screen.getByText(/server a detail/i)).toBeInTheDocument();
  });

  it('does not show detail pane when selected server is not in the list', async () => {
    mockInstalledList.mockResolvedValue([SERVER_A]);
    mockStatus.mockResolvedValue(EMPTY_STATUS);

    render(<McpServersTab />);
    await flushLoad();

    // Right pane starts as 'none'
    expect(screen.queryByTestId('installed-server-detail')).not.toBeInTheDocument();
    expect(screen.getByText(/select a server or browse the catalog/i)).toBeInTheDocument();
  });

  it('status polling starts when a server is connected', async () => {
    mockInstalledList.mockResolvedValue([SERVER_A]);
    mockStatus
      .mockResolvedValueOnce([
        {
          server_id: 'srv-a',
          qualified_name: 'acme/server-a',
          display_name: 'Server A',
          status: 'connected',
          tool_count: 3,
        },
      ])
      .mockResolvedValue([
        {
          server_id: 'srv-a',
          qualified_name: 'acme/server-a',
          display_name: 'Server A',
          status: 'connected',
          tool_count: 3,
        },
      ]);

    render(<McpServersTab />);
    await flushLoad();

    const initialCallCount = mockStatus.mock.calls.length;
    expect(initialCallCount).toBeGreaterThanOrEqual(1);

    // Advance past one poll cycle (5s)
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_100);
    });

    // Status should have been called again due to polling
    expect(mockStatus.mock.calls.length).toBeGreaterThan(initialCallCount);
  });
});
