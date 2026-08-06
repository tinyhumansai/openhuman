import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { StrictMode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Static import — follows project no-dynamic-import rule for test files.
import McpCatalogBrowser from './McpCatalogBrowser';

const mockRegistrySearch = vi.fn();

const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(resolvePromise => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
};

vi.mock('../../../services/api/mcpClientsApi', () => ({
  mcpClientsApi: { registrySearch: (...args: unknown[]) => mockRegistrySearch(...args) },
}));

describe('McpCatalogBrowser', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    mockRegistrySearch.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('renders search input', async () => {
    mockRegistrySearch.mockResolvedValue({ servers: [], page: 1, total_pages: 1 });
    render(<McpCatalogBrowser onSelectInstall={() => {}} />);
    const search = screen.getByPlaceholderText('Search MCP servers...');
    expect(search).toHaveAttribute('type', 'search');
    expect(search).toHaveClass('h-9');
  });

  it('fetches only the latest debounced query and resets pagination', async () => {
    mockRegistrySearch.mockImplementation(({ page }: { page: number }) =>
      Promise.resolve({
        servers: [{ qualified_name: `acme/page-${page}`, display_name: `Page ${page}` }],
        page,
        total_pages: 2,
      })
    );
    render(<McpCatalogBrowser onSelectInstall={() => {}} />);

    await act(async () => {
      await Promise.resolve();
    });
    fireEvent.click(screen.getByRole('button', { name: 'Load more' }));
    expect(mockRegistrySearch).toHaveBeenLastCalledWith({
      query: undefined,
      page: 2,
      page_size: 20,
    });
    mockRegistrySearch.mockClear();

    const input = screen.getByPlaceholderText('Search MCP servers...');
    fireEvent.change(input, { target: { value: 'git' } });
    act(() => vi.advanceTimersByTime(100));
    fireEvent.change(input, { target: { value: 'github' } });

    // Before debounce fires, no new call
    act(() => vi.advanceTimersByTime(249));
    expect(mockRegistrySearch).not.toHaveBeenCalled();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });

    expect(mockRegistrySearch).toHaveBeenCalledTimes(1);
    expect(mockRegistrySearch).toHaveBeenCalledWith({ query: 'github', page: 1, page_size: 20 });
  });

  it('coalesces StrictMode fetches for each stable debounced query', async () => {
    const initialRequest = deferred<{ servers: never[]; page: number; total_pages: number }>();
    const changedRequest = deferred<{ servers: never[]; page: number; total_pages: number }>();
    mockRegistrySearch.mockImplementation(({ query }: { query?: string }) =>
      query === 'github' ? changedRequest.promise : initialRequest.promise
    );

    render(
      <StrictMode>
        <McpCatalogBrowser onSelectInstall={() => {}} />
      </StrictMode>
    );

    initialRequest.resolve({ servers: [], page: 1, total_pages: 1 });
    await act(async () => {
      await initialRequest.promise;
    });
    expect(mockRegistrySearch).toHaveBeenCalledTimes(1);

    fireEvent.change(screen.getByPlaceholderText('Search MCP servers...'), {
      target: { value: 'github' },
    });
    act(() => vi.advanceTimersByTime(249));
    expect(mockRegistrySearch).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(mockRegistrySearch).toHaveBeenCalledTimes(2);
    expect(mockRegistrySearch).toHaveBeenLastCalledWith({
      query: 'github',
      page: 1,
      page_size: 20,
    });

    changedRequest.resolve({ servers: [], page: 1, total_pages: 1 });
    await act(async () => {
      await changedRequest.promise;
    });
  });

  it('renders server cards from search results', async () => {
    const servers = [
      {
        qualified_name: 'acme/file-server',
        display_name: 'File Server',
        description: 'Reads files',
        use_count: 100,
        is_deployed: true,
      },
    ];
    mockRegistrySearch.mockResolvedValue({ servers, page: 1, total_pages: 1 });
    render(<McpCatalogBrowser onSelectInstall={() => {}} />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    // waitFor polls via real setTimeout; switch back so it isn't deadlocked by fake timers.
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.getByText('File Server')).toBeInTheDocument();
    });
    expect(screen.getByText('Reads files')).toBeInTheDocument();
  });

  it('calls onSelectInstall when Install button is clicked', async () => {
    const servers = [{ qualified_name: 'acme/file-server', display_name: 'File Server' }];
    mockRegistrySearch.mockResolvedValue({ servers, page: 1, total_pages: 1 });
    const onSelectInstall = vi.fn();
    render(<McpCatalogBrowser onSelectInstall={onSelectInstall} />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    vi.useRealTimers();

    await waitFor(() => screen.getByText('File Server'));

    fireEvent.click(screen.getByRole('button', { name: /File Server/ }));
    expect(onSelectInstall).toHaveBeenCalledWith('acme/file-server');
  });

  it('shows load more when more pages available', async () => {
    const servers = [{ qualified_name: 'a/b', display_name: 'B' }];
    mockRegistrySearch.mockResolvedValue({ servers, page: 1, total_pages: 3 });
    render(<McpCatalogBrowser onSelectInstall={() => {}} />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    vi.useRealTimers();

    await waitFor(() => screen.getByText('Load more'));
    expect(screen.getByRole('button', { name: 'Load more' })).toBeInTheDocument();
  });

  it('does not crash when registrySearch returns servers: undefined', async () => {
    // Simulates a malformed envelope where the `servers` field is missing.
    // The catalog component spreads `result.servers` — if undefined, the spread
    // would throw. This test verifies a graceful "no results" render instead.
    mockRegistrySearch.mockResolvedValue({ servers: undefined, page: 1, total_pages: 1 });
    render(<McpCatalogBrowser onSelectInstall={() => {}} />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    vi.useRealTimers();

    // Should show empty/no-results state, not crash
    await waitFor(() => {
      expect(screen.getByPlaceholderText('Search MCP servers...')).toBeInTheDocument();
    });
    // No "Install" button — nothing to install from an undefined list
    expect(screen.queryByRole('button', { name: 'Install' })).not.toBeInTheDocument();
  });

  it('does not crash when registrySearch returns null servers', async () => {
    mockRegistrySearch.mockResolvedValue({ servers: null, page: 1, total_pages: 1 });
    render(<McpCatalogBrowser onSelectInstall={() => {}} />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    vi.useRealTimers();

    await waitFor(() => {
      expect(screen.getByPlaceholderText('Search MCP servers...')).toBeInTheDocument();
    });
    expect(screen.queryByRole('button', { name: 'Install' })).not.toBeInTheDocument();
  });

  it('shows friendly guidance when search fails', async () => {
    mockRegistrySearch.mockRejectedValue(
      new Error('MCP official registry returned HTTP 500: {"detail":"upstream down"}')
    );
    render(<McpCatalogBrowser onSelectInstall={() => {}} />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    vi.useRealTimers();

    await waitFor(() => screen.getByText(/The MCP registry is unavailable right now/));
    expect(screen.getByText(/browse available MCP servers/)).toBeInTheDocument();
    expect(screen.queryByText(/"detail":"upstream down"/)).not.toBeInTheDocument();
  });
});
