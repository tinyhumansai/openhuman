import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, type Mock, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import type { GraphExportResponse, GraphNode } from '../../../utils/tauriCommands';
import { MemoryWorkspace } from '../MemoryWorkspace';

// The graph workspace pulls every sealed summary through one RPC call —
// `memory_tree_graph_export`. The MemorySyncConnections poll is mocked
// out separately so the workspace mounts cleanly without hitting the
// network.
vi.mock('../../../utils/tauriCommands', () => ({
  isTauri: vi.fn(() => true),
  memoryTreeGraphExport: vi.fn(),
  memoryTreeFlushNow: vi.fn(),
  memoryTreeWipeAll: vi.fn(),
  memoryTreeResetTree: vi.fn(),
  memoryTreeObsidianVaultStatus: vi.fn(),
}));

vi.mock('../../../services/memorySourcesService', () => ({
  listMemorySources: vi.fn().mockResolvedValue([]),
  memorySourcesStatusList: vi.fn().mockResolvedValue([]),
  syncMemorySource: vi.fn(),
  removeMemorySource: vi.fn(),
  updateMemorySource: vi.fn(),
  addMemorySource: vi.fn(),
  SOURCE_KIND_ICONS: {
    folder: '📁',
    composio: '🔗',
    github_repo: '🐙',
    rss_feed: '📡',
    web_page: '🌐',
    twitter_query: '🐦',
  },
  SOURCE_KIND_LABEL_KEYS: {
    folder: 'memorySources.kind.folder',
    composio: 'memorySources.kind.composio',
    github_repo: 'memorySources.kind.github_repo',
    rss_feed: 'memorySources.kind.rss_feed',
    web_page: 'memorySources.kind.web_page',
    twitter_query: 'memorySources.kind.twitter_query',
  },
}));

vi.mock('../../../lib/composio/composioApi', () => ({
  listConnections: vi.fn().mockResolvedValue({ connections: [] }),
  syncConnection: vi.fn(),
}));

// Stub `openUrl` so deep-link clicks land in a mock instead of routing
// through `tauri-plugin-opener` (which isn't loaded in the test env).
vi.mock('../../../utils/openUrl', () => ({ openUrl: vi.fn().mockResolvedValue(undefined) }));

vi.mock('../../../utils/tauriCommands/workspacePaths', () => ({
  openWorkspacePath: vi.fn().mockResolvedValue(undefined),
  revealWorkspacePath: vi.fn().mockResolvedValue(undefined),
  // #2492: the Obsidian deep link now resolves the vault's absolute path
  // through the shared workspace-link layer instead of trusting the
  // `content_root_abs` field returned from the graph export RPC. Return the
  // same path the prop carries so the existing `openUrl` assertion is stable.
  resolveWorkspaceAbsolutePath: vi.fn().mockResolvedValue('/tmp/workspace/memory_tree/content'),
  previewWorkspaceText: vi
    .fn()
    .mockResolvedValue({
      path: 'memory_tree/content/wiki/summaries/source-alice-x-com/L1/summary-L1-abc.md',
      absolutePath:
        '/tmp/workspace/memory_tree/content/wiki/summaries/source-alice-x-com/L1/summary-L1-abc.md',
      contents: '# Gmail summary',
      truncated: false,
      sizeBytes: 15,
    }),
}));

const {
  memoryTreeGraphExport,
  memoryTreeFlushNow,
  memoryTreeWipeAll,
  memoryTreeResetTree,
  memoryTreeObsidianVaultStatus,
} = (await import('../../../utils/tauriCommands')) as unknown as {
  memoryTreeGraphExport: Mock;
  memoryTreeFlushNow: Mock;
  memoryTreeWipeAll: Mock;
  memoryTreeResetTree: Mock;
  memoryTreeObsidianVaultStatus: Mock;
};

const { listConnections, syncConnection } =
  (await import('../../../lib/composio/composioApi')) as unknown as {
    listConnections: Mock;
    syncConnection: Mock;
  };

const { listMemorySources, syncMemorySource } =
  (await import('../../../services/memorySourcesService')) as unknown as {
    listMemorySources: Mock;
    syncMemorySource: Mock;
  };

const { openUrl } = (await import('../../../utils/openUrl')) as unknown as { openUrl: Mock };

const { openWorkspacePath, revealWorkspacePath } =
  (await import('../../../utils/tauriCommands/workspacePaths')) as unknown as {
    openWorkspacePath: Mock;
    revealWorkspacePath: Mock;
  };

function makeSummary(partial: Partial<GraphNode>): GraphNode {
  return {
    kind: 'summary',
    id: 'summary:L1:abc',
    label: 'L1 · gmail',
    tree_id: 'tree-1',
    tree_kind: 'source',
    tree_scope: 'gmail:alice@x.com',
    level: 1,
    parent_id: null,
    child_count: 4,
    time_range_start_ms: 0,
    time_range_end_ms: 0,
    file_basename: 'summary-L1-abc',
    ...partial,
  };
}

const SAMPLE_RESPONSE: GraphExportResponse = {
  content_root_abs: '/tmp/workspace/memory_tree/content',
  edges: [],
  nodes: [
    makeSummary({ id: 'root', level: 2, parent_id: null, child_count: 2 }),
    makeSummary({ id: 'child-1', level: 1, parent_id: 'root' }),
    makeSummary({ id: 'child-2', level: 1, parent_id: 'root' }),
  ],
};

describe('MemoryWorkspace (graph view)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    memoryTreeGraphExport.mockResolvedValue(SAMPLE_RESPONSE);
    memoryTreeFlushNow.mockResolvedValue({ enqueued: true, stale_buffers: 3 });
    memoryTreeWipeAll.mockResolvedValue({
      rows_deleted: 42,
      dirs_removed: ['raw', 'wiki', 'email'],
      sync_state_cleared: 1,
    });
    memoryTreeResetTree.mockResolvedValue({
      tree_rows_deleted: 12,
      chunks_requeued: 7,
      jobs_enqueued: 7,
    });
    listConnections.mockResolvedValue({ connections: [] });
    syncConnection.mockResolvedValue({ ok: true });
    openUrl.mockResolvedValue(undefined);
    openWorkspacePath.mockResolvedValue(undefined);
    revealWorkspacePath.mockResolvedValue(undefined);
    // Default: the content root is already a registered Obsidian vault, so a
    // View-Vault click opens it directly (the not-registered guidance branch
    // is covered in ObsidianVaultSection.test.tsx).
    memoryTreeObsidianVaultStatus.mockResolvedValue({
      registered: true,
      config_found: true,
      content_root_abs: '/tmp/workspace/memory_tree/content',
    });
  });

  it('renders the SVG graph once the export RPC resolves', async () => {
    renderWithProviders(<MemoryWorkspace />);
    await waitFor(() => {
      expect(screen.getByTestId('memory-graph-svg')).toBeInTheDocument();
    });
    // Three nodes → three circle elements with stable testids.
    expect(screen.getByTestId('memory-graph-node-root')).toBeInTheDocument();
    expect(screen.getByTestId('memory-graph-node-child-1')).toBeInTheDocument();
    expect(screen.getByTestId('memory-graph-node-child-2')).toBeInTheDocument();
  });

  it('shows an empty state when the tree has no sealed summaries', async () => {
    memoryTreeGraphExport.mockResolvedValueOnce({
      content_root_abs: '/tmp/workspace/memory_tree/content',
      edges: [],
      nodes: [],
    });
    renderWithProviders(<MemoryWorkspace />);
    await waitFor(() => {
      expect(screen.getByTestId('memory-graph-empty')).toBeInTheDocument();
    });
  });

  it('"View vault in Obsidian" triggers the deep link via the OS opener (not the webview)', async () => {
    renderWithProviders(<MemoryWorkspace />);
    const button = await screen.findByTestId('memory-open-in-obsidian');
    fireEvent.click(button);
    await waitFor(() => {
      expect(openUrl).toHaveBeenCalledWith(
        'obsidian://open?path=' + encodeURIComponent('/tmp/workspace/memory_tree/content')
      );
    });
  });

  // #2281: every click must produce a visible result. We emit an info
  // toast naming the vault path AND offering a "Reveal Folder" action
  // so users without Obsidian still get feedback + a working escape
  // hatch.
  it('"View vault" click emits an info toast with the vault path and a Reveal Folder action', async () => {
    const onToast = vi.fn();
    renderWithProviders(<MemoryWorkspace onToast={onToast} />);
    const button = await screen.findByTestId('memory-open-in-obsidian');
    fireEvent.click(button);
    await waitFor(() => {
      expect(onToast).toHaveBeenCalled();
    });
    const toast = onToast.mock.calls[0][0];
    expect(toast.type).toBe('info');
    expect(toast.message).toContain('/tmp/workspace/memory_tree/content');
    expect(toast.action?.label).toBeTruthy();
    expect(typeof toast.action?.handler).toBe('function');
  });

  it('Reveal Folder action on the success toast uses the shared workspace reveal command', async () => {
    const onToast = vi.fn();
    renderWithProviders(<MemoryWorkspace onToast={onToast} />);
    fireEvent.click(await screen.findByTestId('memory-open-in-obsidian'));
    await waitFor(() => expect(onToast).toHaveBeenCalled());
    const toast = onToast.mock.calls[0][0];
    toast.action.handler();
    await waitFor(() => {
      expect(revealWorkspacePath).toHaveBeenCalledWith('memory_tree/content');
    });
  });

  it('"View vault" surfaces an error toast (still with Reveal Folder) when openUrl rejects', async () => {
    openUrl.mockRejectedValueOnce(new Error('boom'));
    const onToast = vi.fn();
    renderWithProviders(<MemoryWorkspace onToast={onToast} />);
    fireEvent.click(await screen.findByTestId('memory-open-in-obsidian'));
    await waitFor(() => expect(onToast).toHaveBeenCalled());
    const toast = onToast.mock.calls[0][0];
    expect(toast.type).toBe('error');
    expect(toast.message).toContain('/tmp/workspace/memory_tree/content');
    expect(toast.action?.label).toBeTruthy();
  });

  it('Reveal Folder fallback surfaces an error toast when workspace reveal itself fails', async () => {
    revealWorkspacePath.mockRejectedValueOnce(new Error('reveal failed'));
    const onToast = vi.fn();
    renderWithProviders(<MemoryWorkspace onToast={onToast} />);
    fireEvent.click(await screen.findByTestId('memory-open-in-obsidian'));
    await waitFor(() => expect(onToast).toHaveBeenCalled());
    const firstToast = onToast.mock.calls[0][0];
    firstToast.action.handler();
    await waitFor(() => {
      expect(onToast.mock.calls.length).toBeGreaterThanOrEqual(2);
    });
    const errorToast = onToast.mock.calls[onToast.mock.calls.length - 1][0];
    expect(errorToast.type).toBe('error');
    expect(errorToast.message).toContain('reveal failed');
  });

  it('clicking a summary node opens that file through the shared workspace path command', async () => {
    renderWithProviders(<MemoryWorkspace />);
    const node = await screen.findByTestId('memory-graph-node-child-1');
    fireEvent.click(node);
    const expectedRel = 'wiki/summaries/source-alice-x-com/L1/summary-L1-abc.md';
    await waitFor(() => {
      expect(openWorkspacePath).toHaveBeenCalledWith(`memory_tree/content/${expectedRel}`);
    });
  });

  it('shows sync rows for provider-backed toolkits and hides non-syncable ones', async () => {
    // The new MemorySourcesRegistry reads from listMemorySources (not listConnections).
    // Composio connections are auto-seeded into the registry as MemorySourceEntry records.
    listMemorySources.mockResolvedValue([
      { id: 'src-gmail', kind: 'composio', toolkit: 'gmail', label: 'Gmail · a@x', enabled: true },
      { id: 'src-slack', kind: 'composio', toolkit: 'slack', label: 'Slack · acme', enabled: true },
      { id: 'src-notion', kind: 'composio', toolkit: 'notion', label: 'Notion', enabled: true },
    ]);
    renderWithProviders(<MemoryWorkspace />);
    // Provider-backed toolkits should render actionable Sync rows
    expect(await screen.findByTestId('memory-source-sync-gmail')).toBeInTheDocument();
    expect(screen.getByTestId('memory-source-sync-slack')).toBeInTheDocument();
    expect(screen.getByTestId('memory-source-sync-notion')).toBeInTheDocument();
    // Discord was not added as a source, so no row exists for it.
    expect(screen.queryAllByTestId('memory-source-row-composio').length).toBeGreaterThan(0); // some composio rows exist
    expect(screen.queryByTestId('memory-source-sync-discord')).toBeNull();
  });

  it('toggling to Contacts mode re-fetches the graph with mode=contacts', async () => {
    renderWithProviders(<MemoryWorkspace />);
    await screen.findByTestId('memory-graph-svg');
    expect(memoryTreeGraphExport).toHaveBeenLastCalledWith('tree');
    const contactsTab = screen.getByTestId('memory-graph-mode-contacts');
    fireEvent.click(contactsTab);
    await waitFor(() => {
      expect(memoryTreeGraphExport).toHaveBeenLastCalledWith('contacts');
    });
  });

  it('"Reset memory" requires a confirm and then dispatches memory_tree_wipe_all', async () => {
    const onToast = vi.fn();
    const confirmSpy = vi.spyOn(window, 'confirm');
    // First click — user cancels the confirm dialog → no RPC call.
    confirmSpy.mockReturnValueOnce(false);
    renderWithProviders(<MemoryWorkspace onToast={onToast} />);
    const button = await screen.findByTestId('memory-wipe-all');
    fireEvent.click(button);
    await waitFor(() => {
      expect(confirmSpy).toHaveBeenCalledTimes(1);
    });
    expect(memoryTreeWipeAll).not.toHaveBeenCalled();

    // Second click — user accepts. RPC fires, success toast carries
    // the rows count, and the graph re-fetches.
    confirmSpy.mockReturnValueOnce(true);
    fireEvent.click(button);
    await waitFor(() => {
      expect(memoryTreeWipeAll).toHaveBeenCalledTimes(1);
    });
    await waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'success',
          title: 'Memory wiped',
          message: expect.stringContaining('42'),
        })
      );
    });
    confirmSpy.mockRestore();
  });

  it('"Reset memory tree" requires a confirm and dispatches memory_tree_reset_tree', async () => {
    const onToast = vi.fn();
    const confirmSpy = vi.spyOn(window, 'confirm');

    // Cancel first → no RPC call.
    confirmSpy.mockReturnValueOnce(false);
    renderWithProviders(<MemoryWorkspace onToast={onToast} />);
    const button = await screen.findByTestId('memory-reset-tree');
    fireEvent.click(button);
    await waitFor(() => {
      expect(confirmSpy).toHaveBeenCalledTimes(1);
    });
    expect(memoryTreeResetTree).not.toHaveBeenCalled();

    // Accept → RPC fires, success toast carries the chunk + job counts.
    confirmSpy.mockReturnValueOnce(true);
    fireEvent.click(button);
    await waitFor(() => {
      expect(memoryTreeResetTree).toHaveBeenCalledTimes(1);
    });
    await waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'success',
          title: 'Memory tree rebuilding',
          message: expect.stringContaining('7'),
        })
      );
    });
    confirmSpy.mockRestore();
  });

  it('"Build summary trees" calls memory_tree_flush_now and toasts the buffer count', async () => {
    const onToast = vi.fn();
    renderWithProviders(<MemoryWorkspace onToast={onToast} />);
    const button = await screen.findByTestId('memory-build-trees');
    fireEvent.click(button);
    await waitFor(() => {
      expect(memoryTreeFlushNow).toHaveBeenCalledTimes(1);
    });
    await waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'success', title: expect.stringContaining('3 buffer') })
      );
    });
  });

  it('per-source Sync button dispatches memory_sources_sync with the source id', async () => {
    listMemorySources.mockResolvedValue([
      {
        id: 'src-gmail-001',
        kind: 'composio',
        toolkit: 'gmail',
        label: 'Gmail · alice@example.com',
        enabled: true,
      },
    ]);
    syncMemorySource.mockResolvedValue(undefined);
    const onToast = vi.fn();
    renderWithProviders(<MemoryWorkspace onToast={onToast} />);
    const button = await screen.findByTestId('memory-source-sync-gmail');
    // Source row title surfaces the account identity.
    expect(button.closest('li')).toHaveTextContent(/Gmail · alice@example\.com/);
    fireEvent.click(button);
    await waitFor(() => {
      expect(syncMemorySource).toHaveBeenCalledWith('src-gmail-001');
    });
    await waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'success',
          title: expect.stringContaining('alice@example.com'),
        })
      );
    });
  });

  it('surfaces an error message when the export RPC rejects', async () => {
    memoryTreeGraphExport.mockRejectedValueOnce(new Error('boom'));
    renderWithProviders(<MemoryWorkspace />);
    await waitFor(() => {
      expect(screen.getByText(/Failed to load memory graph/)).toBeInTheDocument();
    });
  });
});
