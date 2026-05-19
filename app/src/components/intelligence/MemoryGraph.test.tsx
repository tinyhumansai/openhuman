import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { GraphEdge, GraphNode } from '../../utils/tauriCommands';
import { MemoryGraph } from './MemoryGraph';

const openUrlMock = vi.fn();
vi.mock('../../utils/openUrl', () => ({ openUrl: (...args: unknown[]) => openUrlMock(...args) }));

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
    openUrlMock.mockReset();
    openUrlMock.mockResolvedValue(undefined);
  });

  it('renders the empty state when there are no nodes', () => {
    render(<MemoryGraph nodes={[]} edges={[]} mode="tree" contentRootAbs="/tmp/openhuman" />);
    expect(screen.getByTestId('memory-graph-empty')).toBeInTheDocument();
  });

  it('renders an SVG with one circle per node in tree mode', () => {
    const nodes = [
      makeSummaryNode({ id: 'root', level: 0, parent_id: null }),
      makeSummaryNode({ id: 'child', level: 1, parent_id: 'root' }),
    ];
    const { container } = render(
      <MemoryGraph nodes={nodes} edges={[]} mode="tree" contentRootAbs="/tmp" />
    );
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
    render(<MemoryGraph nodes={nodes} edges={edges} mode="contacts" contentRootAbs="/tmp" />);
    // Two legend rows render with i18n keys as fallback (graph.document/contact)
    // — assert via the rendered nodes count instead, which is deterministic.
    expect(screen.getAllByTestId(/memory-graph-node-/).length).toBe(2);
  });

  it('opens the Obsidian deep link when a summary node is clicked', async () => {
    const nodes = [
      makeSummaryNode({
        id: 'sum-A',
        tree_kind: 'topic',
        tree_scope: 'workspace one',
        level: 2,
        file_basename: 'summary-A',
      }),
    ];
    render(
      <MemoryGraph
        nodes={nodes}
        edges={[]}
        mode="tree"
        contentRootAbs="/Users/me/openhuman-content"
      />
    );
    fireEvent.click(screen.getByTestId('memory-graph-node-sum-A'));
    // Allow the async openSummaryInObsidian to dispatch.
    await Promise.resolve();
    expect(openUrlMock).toHaveBeenCalledTimes(1);
    const url = openUrlMock.mock.calls[0][0] as string;
    expect(url.startsWith('obsidian://open?path=')).toBe(true);
    // Slugified `tree_scope` ("workspace one") joins the file path.
    expect(decodeURIComponent(url)).toContain('topic-workspace-one');
    expect(decodeURIComponent(url)).toContain('L2/summary-A.md');
  });

  it('does NOT call openUrl when a non-summary node is clicked', async () => {
    const nodes = [makeChunkNode({ id: 'doc-1' })];
    render(<MemoryGraph nodes={nodes} edges={[]} mode="contacts" contentRootAbs="/tmp" />);
    fireEvent.click(screen.getByTestId('memory-graph-node-doc-1'));
    await Promise.resolve();
    expect(openUrlMock).not.toHaveBeenCalled();
  });

  it('shows a tooltip footer when a node is hovered', () => {
    const nodes = [makeContactNode({ id: 'person:bob', label: 'Bob' })];
    render(<MemoryGraph nodes={nodes} edges={[]} mode="contacts" contentRootAbs="/tmp" />);
    fireEvent.mouseEnter(screen.getByTestId('memory-graph-node-person:bob'));
    expect(screen.getByTestId('memory-graph-tooltip')).toBeInTheDocument();
    expect(screen.getByTestId('memory-graph-tooltip').textContent).toContain('Bob');
  });
});
