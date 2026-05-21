import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Static import — follows project no-dynamic-import rule for test files.
import McpCatalogBrowser from './McpCatalogBrowser';

const mockRegistrySearch = vi.fn();

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
    expect(screen.getByPlaceholderText('Search Smithery catalog...')).toBeInTheDocument();
  });

  it('fires debounced search on input change', async () => {
    mockRegistrySearch.mockResolvedValue({ servers: [], page: 1, total_pages: 1 });
    render(<McpCatalogBrowser onSelectInstall={() => {}} />);

    const input = screen.getByPlaceholderText('Search Smithery catalog...');

    // Advance past the initial debounce
    await act(async () => {
      vi.advanceTimersByTime(300);
    });
    mockRegistrySearch.mockClear();

    // Type in the search box
    fireEvent.change(input, { target: { value: 'github' } });

    // Before debounce fires, no new call
    expect(mockRegistrySearch).not.toHaveBeenCalled();

    // Advance past the 250ms debounce
    await act(async () => {
      vi.advanceTimersByTime(300);
    });

    expect(mockRegistrySearch).toHaveBeenCalledWith({ query: 'github', page: 1, page_size: 20 });
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

    fireEvent.click(screen.getByRole('button', { name: 'Install' }));
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

  it('shows error state when search fails', async () => {
    mockRegistrySearch.mockRejectedValue(new Error('Network error'));
    render(<McpCatalogBrowser onSelectInstall={() => {}} />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    vi.useRealTimers();

    await waitFor(() => screen.getByText('Network error'));
  });
});
