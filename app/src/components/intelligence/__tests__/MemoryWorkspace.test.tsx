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
}));

vi.mock('../../../services/memorySyncService', () => ({
  memorySyncStatusList: vi.fn().mockResolvedValue([]),
}));

vi.mock('../../../lib/composio/composioApi', () => ({
  listConnections: vi.fn().mockResolvedValue({ connections: [] }),
  syncConnection: vi.fn(),
}));

const { memoryTreeGraphExport, memoryTreeFlushNow } = (await import(
  '../../../utils/tauriCommands'
)) as unknown as {
  memoryTreeGraphExport: Mock;
  memoryTreeFlushNow: Mock;
};

const { listConnections, syncConnection } = (await import(
  '../../../lib/composio/composioApi'
)) as unknown as {
  listConnections: Mock;
  syncConnection: Mock;
};

function makeNode(partial: Partial<GraphNode>): GraphNode {
  return {
    id: 'summary:L1:abc',
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
  nodes: [
    makeNode({ id: 'root', level: 2, parent_id: null, child_count: 2 }),
    makeNode({ id: 'child-1', level: 1, parent_id: 'root' }),
    makeNode({ id: 'child-2', level: 1, parent_id: 'root' }),
  ],
};

describe('MemoryWorkspace (graph view)', () => {
  let originalLocation: Location;

  beforeEach(() => {
    vi.clearAllMocks();
    memoryTreeGraphExport.mockResolvedValue(SAMPLE_RESPONSE);
    memoryTreeFlushNow.mockResolvedValue({ enqueued: true, stale_buffers: 3 });
    listConnections.mockResolvedValue({ connections: [] });
    syncConnection.mockResolvedValue({ ok: true });
    // Stub `window.location.href` so the deep-link click is observable
    // without actually navigating away during the test run.
    originalLocation = window.location;
    Object.defineProperty(window, 'location', {
      writable: true,
      value: { ...originalLocation, href: '' },
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
      nodes: [],
    });
    renderWithProviders(<MemoryWorkspace />);
    await waitFor(() => {
      expect(screen.getByTestId('memory-graph-empty')).toBeInTheDocument();
    });
  });

  it('"View vault in Obsidian" triggers an obsidian:// deep link to the content root', async () => {
    renderWithProviders(<MemoryWorkspace />);
    const button = await screen.findByTestId('memory-open-in-obsidian');
    fireEvent.click(button);
    expect(window.location.href).toBe(
      'obsidian://open?path=' +
        encodeURIComponent('/tmp/workspace/memory_tree/content')
    );
  });

  it('clicking a graph node opens that summary in Obsidian via the deep link', async () => {
    renderWithProviders(<MemoryWorkspace />);
    const node = await screen.findByTestId('memory-graph-node-child-1');
    fireEvent.click(node);
    // child-1 has tree_kind=source, level=1, scope=gmail:alice@x.com →
    // slug "gmail-alice-x-com", basename "summary-L1-abc".
    const expectedRel =
      'summaries/source/gmail-alice-x-com/L1/summary-L1-abc.md';
    const expectedAbs = '/tmp/workspace/memory_tree/content/' + expectedRel;
    expect(window.location.href).toBe(
      'obsidian://open?path=' + encodeURIComponent(expectedAbs)
    );
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
        expect.objectContaining({
          type: 'success',
          title: expect.stringContaining('3 buffer'),
        })
      );
    });
  });

  it('per-connection Sync button dispatches composio.sync with the connection id', async () => {
    listConnections.mockResolvedValue({
      connections: [
        {
          id: 'conn-gmail-001',
          toolkit: 'gmail',
          status: 'ACTIVE',
          accountEmail: 'alice@example.com',
        },
      ],
    });
    const onToast = vi.fn();
    renderWithProviders(<MemoryWorkspace onToast={onToast} />);
    const button = await screen.findByTestId('memory-source-sync-gmail');
    fireEvent.click(button);
    await waitFor(() => {
      expect(syncConnection).toHaveBeenCalledWith('conn-gmail-001', 'manual');
    });
    await waitFor(() => {
      expect(onToast).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'success',
          title: expect.stringContaining('Gmail'),
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
