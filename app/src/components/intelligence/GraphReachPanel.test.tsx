import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { computeGraphReach } from '../../lib/memory/graphReach';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import GraphReachPanel from './GraphReachPanel';

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

// Path A-B-C-D: diameter 3, radius 2, center {B,C}.
const path = computeGraphReach([rel('A', 'B'), rel('B', 'C'), rel('C', 'D')]);

describe('<GraphReachPanel />', () => {
  it('renders the loading skeleton', () => {
    render(<GraphReachPanel result={null} loading />);
    expect(screen.getByTestId('graph-reach-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there are no nodes', () => {
    render(<GraphReachPanel result={computeGraphReach([])} />);
    expect(screen.getByText('No knowledge graph yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(<GraphReachPanel result={null} error="graph unavailable" onRetry={onRetry} />);
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('renders metric tiles, the component summary, and the ranked table', () => {
    render(<GraphReachPanel result={path} />);
    expect(screen.getByText('Entities')).toBeInTheDocument();
    expect(screen.getByText('Diameter')).toBeInTheDocument();
    expect(screen.getByText('Radius')).toBeInTheDocument();
    expect(screen.getByText('Most central entities')).toBeInTheDocument();
    // single component holding all four nodes -> singular caption variant.
    expect(screen.getByText('1 component · 4 entities')).toBeInTheDocument();
  });

  it('badges the centers (eccentricity == radius) and not the periphery', () => {
    render(<GraphReachPanel result={path} />);
    // B and C are the two centers of the path; A and D are not.
    expect(screen.getAllByText('center')).toHaveLength(2);
  });

  it('uses the plural caption when the graph has more than one component', () => {
    // Path P-Q-R-S (giant, size 4) plus disjoint edge Y-Z (size 2).
    const multi = computeGraphReach([rel('P', 'Q'), rel('Q', 'R'), rel('R', 'S'), rel('Y', 'Z')]);
    render(<GraphReachPanel result={multi} />);
    expect(screen.getByText('2 components · largest holds 4')).toBeInTheDocument();
  });

  it('uses the all-singular caption for a single-node component (self-loop-only)', () => {
    // The only fact is "Alice→Alice": the engine keeps Alice as a singleton
    // (size 1), and the caption renders the all-singular variant — never the
    // ungrammatical "1 component · 1 entities".
    const lonely = computeGraphReach([rel('Alice', 'Alice')]);
    render(<GraphReachPanel result={lonely} />);
    expect(screen.getByText('1 component · 1 entity')).toBeInTheDocument();
  });
});
