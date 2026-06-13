import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { computeGraphCohesion } from '../../lib/memory/graphCohesion';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import GraphCohesionPanel from './GraphCohesionPanel';

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

// Diamond: A-B, A-C, B-C, B-D, C-D. avg clustering 5/6 (0.83), transitivity 0.75.
const diamond = computeGraphCohesion([
  rel('A', 'B'),
  rel('A', 'C'),
  rel('B', 'C'),
  rel('B', 'D'),
  rel('C', 'D'),
]);

describe('<GraphCohesionPanel />', () => {
  it('renders the loading skeleton', () => {
    render(<GraphCohesionPanel result={null} loading />);
    expect(screen.getByTestId('graph-cohesion-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there are no nodes', () => {
    render(<GraphCohesionPanel result={computeGraphCohesion([])} />);
    expect(screen.getByText('No knowledge graph yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(<GraphCohesionPanel result={null} error="graph unavailable" onRetry={onRetry} />);
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('renders metric tiles, the network averages, and the brokerage ranking', () => {
    render(<GraphCohesionPanel result={diamond} />);
    expect(screen.getByText('Entities')).toBeInTheDocument();
    expect(screen.getByText('Connections')).toBeInTheDocument();
    expect(screen.getByText('Triangles')).toBeInTheDocument();
    expect(screen.getByText('Brokers — loosest neighbourhoods')).toBeInTheDocument();
    // average clustering 5/6 -> 0.83, transitivity 0.75.
    expect(screen.getByText(/transitivity 0\.75/)).toBeInTheDocument();
    // the spine nodes B and C cluster at 0.67 (rendered to 2 decimals).
    expect(screen.getAllByText('0.67')).toHaveLength(2);
  });

  it('badges a structural hole (clustering 0) as a broker', () => {
    // Star: X links A, B, C which never connect -> X is a pure broker.
    const star = computeGraphCohesion([rel('X', 'A'), rel('X', 'B'), rel('X', 'C')]);
    render(<GraphCohesionPanel result={star} />);
    expect(screen.getByText('broker')).toBeInTheDocument();
    expect(screen.getByText('X')).toBeInTheDocument();
  });

  it('shows the no-brokers note when every entity has fewer than two links', () => {
    const single = computeGraphCohesion([rel('A', 'B')]);
    render(<GraphCohesionPanel result={single} />);
    expect(screen.getByText('No entities with two or more connections yet.')).toBeInTheDocument();
    // tiles still render (the graph is non-empty), but no ranking table.
    expect(screen.getByText('Entities')).toBeInTheDocument();
    expect(screen.queryByText('Brokers — loosest neighbourhoods')).not.toBeInTheDocument();
  });
});
