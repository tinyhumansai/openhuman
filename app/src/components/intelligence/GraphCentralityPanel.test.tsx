import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { computeGraphCentrality } from '../../lib/memory/graphCentrality';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import GraphCentralityPanel from './GraphCentralityPanel';

function rel(subject: string, object: string, evidenceCount = 1): GraphRelation {
  return {
    namespace: 'n',
    subject,
    predicate: 'p',
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

const fixtureE = computeGraphCentrality([
  rel('A', 'B'),
  rel('A', 'C'),
  rel('B', 'C'),
  rel('C', 'A'),
  rel('D', 'C'),
]);

describe('<GraphCentralityPanel />', () => {
  it('renders the loading skeleton', () => {
    render(<GraphCentralityPanel result={null} loading />);
    expect(screen.getByTestId('graph-centrality-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there are no nodes', () => {
    render(<GraphCentralityPanel result={computeGraphCentrality([])} />);
    expect(screen.getByText('No knowledge graph yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(<GraphCentralityPanel result={null} error="graph unavailable" onRetry={onRetry} />);
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('renders metric tiles and the ranked hub table for a populated graph', () => {
    render(<GraphCentralityPanel result={fixtureE} />);
    expect(screen.getByText('Entities')).toBeInTheDocument();
    expect(screen.getByText('Connections')).toBeInTheDocument();
    expect(screen.getByText('Clusters')).toBeInTheDocument();
    expect(screen.getByText('Top entities by influence')).toBeInTheDocument();
    // C is the top hub; its PageRank (0.394) renders to 3 decimals.
    expect(screen.getByText('C')).toBeInTheDocument();
    expect(screen.getByText('0.394')).toBeInTheDocument();
  });

  it('flags a connector/bridge entity with a badge', () => {
    // V inherits hub H's outflow (top influence) but has degree 1, while the
    // source hubs M2/M3 fill the middle degree tiers -> V is a connector.
    const bridgeResult = computeGraphCentrality([
      rel('L1', 'H'),
      rel('L2', 'H'),
      rel('L3', 'H'),
      rel('H', 'V'),
      rel('M2', 'x'),
      rel('M2', 'y'),
      rel('M3', 'p'),
      rel('M3', 'q'),
      rel('M3', 'r'),
    ]);
    render(<GraphCentralityPanel result={bridgeResult} />);
    expect(screen.getByText('connector')).toBeInTheDocument();
  });
});
