import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { computeGraphBridges } from '../../lib/memory/graphBridges';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import GraphBridgesPanel from './GraphBridgesPanel';

function rel(subject: string, object: string): GraphRelation {
  return {
    namespace: 'n',
    subject,
    predicate: 'p',
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

// Two triangles joined by a single edge: C and D are articulations; (C,D) is
// the only bridge. articulationCount=2, bridges=[{a:'C',b:'D'}].
const joined = computeGraphBridges([
  rel('A', 'B'),
  rel('B', 'C'),
  rel('C', 'A'),
  rel('D', 'E'),
  rel('E', 'F'),
  rel('F', 'D'),
  rel('C', 'D'),
]);

describe('<GraphBridgesPanel />', () => {
  it('renders the loading skeleton', () => {
    render(<GraphBridgesPanel result={null} loading />);
    expect(screen.getByTestId('graph-bridges-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there are no nodes', () => {
    render(<GraphBridgesPanel result={computeGraphBridges([])} />);
    expect(screen.getByText('No knowledge graph yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(<GraphBridgesPanel result={null} error="graph unavailable" onRetry={onRetry} />);
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('renders metric tiles, articulation badges, and the bridge list', () => {
    render(<GraphBridgesPanel result={joined} />);
    expect(screen.getByText('Entities')).toBeInTheDocument();
    expect(screen.getByText('Connections')).toBeInTheDocument();
    expect(screen.getByText('Articulations')).toBeInTheDocument();
    expect(screen.getByText('Articulation entities')).toBeInTheDocument();
    expect(screen.getByText('Bridge relations')).toBeInTheDocument();
    // both articulation badges present.
    expect(screen.getAllByText('cut vertex')).toHaveLength(2);
    // single bridge row C — D: each id appears in BOTH the articulation table
    // row and the bridge list, so use getAllByText with the matching length.
    expect(screen.getAllByText('C')).toHaveLength(2);
    expect(screen.getAllByText('D')).toHaveLength(2);
    // exactly one bridge and one connected component -> the singular-bridge,
    // singular-component caption is rendered (no "1 bridges" ungrammar).
    expect(screen.getByText('1 bridge · single component')).toBeInTheDocument();
  });

  it('uses the plural caption when there are 2+ bridges and 2+ components', () => {
    // Path A-B-C (2 bridges, 1 component) plus disconnected Y-Z (1 bridge, 2nd
    // component). Total: 3 bridges across 2 components -> fully plural caption.
    const multi = computeGraphBridges([rel('A', 'B'), rel('B', 'C'), rel('Y', 'Z')]);
    render(<GraphBridgesPanel result={multi} />);
    expect(screen.getByText('3 bridges · 2 components')).toBeInTheDocument();
  });

  it('shows the no-fragiles / no-bridges notes for a fully-cyclic graph', () => {
    const triangle = computeGraphBridges([rel('A', 'B'), rel('B', 'C'), rel('C', 'A')]);
    render(<GraphBridgesPanel result={triangle} />);
    expect(
      screen.getByText(
        'No structural single-points-of-failure — every link sits in at least one cycle.'
      )
    ).toBeInTheDocument();
    expect(
      screen.getByText('No bridges — every relation sits in at least one cycle.')
    ).toBeInTheDocument();
  });
});
