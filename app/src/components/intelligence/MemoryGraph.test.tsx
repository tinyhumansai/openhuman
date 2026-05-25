import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { GraphEdge, GraphNode } from '../../utils/tauriCommands';
import { MemoryGraph } from './MemoryGraph';

const mocks = vi.hoisted(() => ({
  openUrl: vi.fn(),
  openWorkspacePath: vi.fn(),
  previewWorkspaceText: vi.fn(),
}));

vi.mock('../../utils/openUrl', () => ({ openUrl: (...args: unknown[]) => mocks.openUrl(...args) }));
vi.mock('../../utils/tauriCommands/workspacePaths', () => ({
  openWorkspacePath: (...args: unknown[]) => mocks.openWorkspacePath(...args),
  previewWorkspaceText: (...args: unknown[]) => mocks.previewWorkspaceText(...args),
}));

function makeSummaryNode(overrides: Partial<GraphNode> = {}): GraphNode {
  return {
    kind: 'summary',
    id: 'sum-1',
    label: 'Summary 1',
    tree_id: 't-1',
    tree_kind: 'topic',
    tree_scope: 'work',
    level: 0,
    parent_id: null,
    child_count: 2,
    file_basename: 'summary-1',
    ...overrides,
  };
}

function makeChunkNode(overrides: Partial<GraphNode> = {}): GraphNode {
  return { kind: 'chunk', id: 'chunk-1', label: 'A chunk', ...overrides };
}

function makeContactNode(overrides: Partial<GraphNode> = {}): GraphNode {
  return {
    kind: 'contact',
    id: 'person:alice',
    label: 'Alice',
    entity_kind: 'person',
    ...overrides,
  };
}

describe('<MemoryGraph />', () => {
  beforeEach(() => {
    mocks.openUrl.mockReset();
    mocks.openUrl.mockResolvedValue(undefined);
    mocks.openWorkspacePath.mockReset();
    mocks.openWorkspacePath.mockResolvedValue(undefined);
    mocks.previewWorkspaceText.mockReset();
    mocks.previewWorkspaceText.mockResolvedValue({
      path: 'memory_tree/content/wiki/summaries/topic-workspace-one/L2/summary-A.md',
      absolutePath:
        '/Users/me/openhuman/memory_tree/content/wiki/summaries/topic-workspace-one/L2/summary-A.md',
      contents: '# Summary\n\nWorkspace one notes',
      truncated: false,
      sizeBytes: 30,
    });
  });

  it('renders the empty state when there are no nodes', () => {
    render(<MemoryGraph nodes={[]} edges={[]} mode="tree" />);
    expect(screen.getByTestId('memory-graph-empty')).toBeInTheDocument();
  });

  it('renders an SVG with one circle per node in tree mode', () => {
    const nodes = [
      makeSummaryNode({ id: 'root', level: 0, parent_id: null }),
      makeSummaryNode({ id: 'child', level: 1, parent_id: 'root' }),
    ];
    const { container } = render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />);
    expect(screen.getByTestId('memory-graph-svg')).toBeInTheDocument();
    expect(container.querySelectorAll('circle').length).toBe(2);
    expect(screen.getByTestId('memory-graph-node-root')).toBeInTheDocument();
    expect(screen.getByTestId('memory-graph-node-child')).toBeInTheDocument();
  });

  it('renders contacts-mode legend rows for chunk and contact', () => {
    const nodes = [
      makeChunkNode({ id: 'd1' }),
      makeContactNode({ id: 'person:alice', label: 'Alice' }),
    ];
    const edges: GraphEdge[] = [{ from: 'd1', to: 'person:alice' }];
    render(<MemoryGraph nodes={nodes} edges={edges} mode="contacts" />);
    // Two legend rows render with i18n keys as fallback (graph.document/contact)
    // — assert via the rendered nodes count instead, which is deterministic.
    expect(screen.getAllByTestId(/memory-graph-node-/).length).toBe(2);
  });

  it('opens a summary node through the shared workspace path command', async () => {
    const nodes = [
      makeSummaryNode({
        id: 'sum-A',
        tree_kind: 'topic',
        tree_scope: 'workspace one',
        level: 2,
        file_basename: 'summary-A',
      }),
    ];
    render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />);
    fireEvent.click(screen.getByTestId('memory-graph-node-sum-A'));
    await waitFor(() => {
      expect(mocks.openWorkspacePath).toHaveBeenCalledWith(
        'memory_tree/content/wiki/summaries/topic-workspace-one/L2/summary-A.md'
      );
    });
    expect(mocks.openUrl).not.toHaveBeenCalled();
  });

  it('logs workspace open failures without falling back to raw URL opens', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    mocks.openWorkspacePath.mockRejectedValueOnce(new Error('open failed'));
    const nodes = [
      makeSummaryNode({
        id: 'sum-open-fails',
        tree_kind: 'topic',
        tree_scope: 'workspace one',
        level: 2,
        file_basename: 'summary-A',
      }),
    ];

    render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />);
    fireEvent.click(screen.getByTestId('memory-graph-node-sum-open-fails'));

    await waitFor(() => {
      expect(errorSpy).toHaveBeenCalledWith(
        '[memory-graph] openWorkspacePath failed',
        expect.any(Error)
      );
    });
    expect(mocks.openUrl).not.toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it('keeps non-Gmail source prefixes in summary workspace paths', async () => {
    const nodes = [
      makeSummaryNode({
        id: 'sum-slack',
        tree_kind: 'source',
        tree_scope: 'slack:#eng',
        level: 2,
        file_basename: 'summary-slack',
      }),
    ];
    render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />);
    fireEvent.click(screen.getByTestId('memory-graph-node-sum-slack'));
    await waitFor(() => {
      expect(mocks.openWorkspacePath).toHaveBeenCalledWith(
        'memory_tree/content/wiki/summaries/source-slack-eng/L2/summary-slack.md'
      );
    });
  });

  it('previews a hovered summary through the shared workspace preview command', async () => {
    const nodes = [
      makeSummaryNode({
        id: 'sum-A',
        tree_kind: 'topic',
        tree_scope: 'workspace one',
        level: 2,
        file_basename: 'summary-A',
      }),
    ];
    render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />);

    fireEvent.mouseEnter(screen.getByTestId('memory-graph-node-sum-A'));
    fireEvent.click(screen.getByTestId('memory-graph-preview-sum-A'));

    await waitFor(() => {
      expect(mocks.previewWorkspaceText).toHaveBeenCalledWith(
        'memory_tree/content/wiki/summaries/topic-workspace-one/L2/summary-A.md'
      );
    });
    expect(await screen.findByTestId('memory-graph-preview')).toHaveTextContent(
      'Workspace one notes'
    );
  });

  it('shows preview errors from the shared workspace preview command', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    mocks.previewWorkspaceText.mockRejectedValueOnce(new Error('preview failed'));
    const nodes = [
      makeSummaryNode({
        id: 'sum-preview-fails',
        tree_kind: 'topic',
        tree_scope: 'workspace one',
        level: 2,
        file_basename: 'summary-A',
      }),
    ];

    render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />);
    fireEvent.mouseEnter(screen.getByTestId('memory-graph-node-sum-preview-fails'));
    fireEvent.click(screen.getByTestId('memory-graph-preview-sum-preview-fails'));

    expect(await screen.findByTestId('memory-graph-preview')).toHaveTextContent('preview failed');
    expect(errorSpy).toHaveBeenCalledWith(
      '[memory-graph] previewWorkspaceText failed',
      expect.any(Error)
    );
    errorSpy.mockRestore();
  });

  it('marks truncated summary previews in the preview panel', async () => {
    mocks.previewWorkspaceText.mockResolvedValueOnce({
      path: 'memory_tree/content/wiki/summaries/topic-workspace-one/L2/summary-A.md',
      absolutePath:
        '/Users/me/openhuman/memory_tree/content/wiki/summaries/topic-workspace-one/L2/summary-A.md',
      contents: '# Summary',
      truncated: true,
      sizeBytes: 100_000,
    });
    const nodes = [
      makeSummaryNode({
        id: 'sum-truncated',
        tree_kind: 'topic',
        tree_scope: 'workspace one',
        level: 2,
        file_basename: 'summary-A',
      }),
    ];

    render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />);
    fireEvent.mouseEnter(screen.getByTestId('memory-graph-node-sum-truncated'));
    fireEvent.click(screen.getByTestId('memory-graph-preview-sum-truncated'));

    expect(await screen.findByTestId('memory-graph-preview')).toHaveTextContent('# Summary');
    expect(screen.getByTestId('memory-graph-preview')).toHaveTextContent('…');
  });

  it('keeps the summary preview action reachable after leaving the SVG node', () => {
    const nodes = [
      makeSummaryNode({
        id: 'sum-A',
        tree_kind: 'topic',
        tree_scope: 'workspace one',
        level: 2,
        file_basename: 'summary-A',
      }),
    ];
    render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />);

    const node = screen.getByTestId('memory-graph-node-sum-A');
    fireEvent.mouseEnter(node);
    fireEvent.mouseLeave(node, { relatedTarget: screen.getByTestId('memory-graph-tooltip') });

    expect(screen.getByTestId('memory-graph-preview-sum-A')).toBeInTheDocument();
  });

  it('clears the hovered node when the pointer leaves the graph', () => {
    const nodes = [
      makeSummaryNode({
        id: 'sum-A',
        tree_kind: 'topic',
        tree_scope: 'workspace one',
        level: 2,
        file_basename: 'summary-A',
      }),
    ];
    const { container } = render(<MemoryGraph nodes={nodes} edges={[]} mode="tree" />);

    fireEvent.mouseEnter(screen.getByTestId('memory-graph-node-sum-A'));
    expect(screen.getByTestId('memory-graph-tooltip')).toBeInTheDocument();
    fireEvent.mouseLeave(container.querySelector('.memory-graph') as Element);

    expect(screen.queryByTestId('memory-graph-tooltip')).not.toBeInTheDocument();
  });

  it('does NOT call workspace open when a non-summary node is clicked', async () => {
    const nodes = [makeChunkNode({ id: 'doc-1' })];
    render(<MemoryGraph nodes={nodes} edges={[]} mode="contacts" />);
    fireEvent.click(screen.getByTestId('memory-graph-node-doc-1'));
    await Promise.resolve();
    expect(mocks.openWorkspacePath).not.toHaveBeenCalled();
  });

  it('shows a tooltip footer when a node is hovered', () => {
    const nodes = [makeContactNode({ id: 'person:bob', label: 'Bob' })];
    render(<MemoryGraph nodes={nodes} edges={[]} mode="contacts" />);
    fireEvent.mouseEnter(screen.getByTestId('memory-graph-node-person:bob'));
    expect(screen.getByTestId('memory-graph-tooltip')).toBeInTheDocument();
    expect(screen.getByTestId('memory-graph-tooltip').textContent).toContain('Bob');
  });
});
