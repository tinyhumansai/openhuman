import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { computePredicateBundles } from '../../lib/memory/predicateBundles';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import PredicateBundlesPanel from './PredicateBundlesPanel';

function rel(subject: string, predicate: string, object: string): GraphRelation {
  return {
    namespace: 'n',
    subject,
    predicate,
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

// 1 thick pair (A-B: 3 predicates × 1 dup) + 1 thin pair (C-D) + duplicate C-D
const mixed = computePredicateBundles([
  rel('A', 'knows', 'B'),
  rel('A', 'trusts', 'B'),
  rel('A', 'mentors', 'B'),
  rel('C', 'knows', 'D'),
  rel('C', 'knows', 'D'), // dup of thin pair
]);

describe('<PredicateBundlesPanel />', () => {
  it('renders the loading skeleton', () => {
    render(<PredicateBundlesPanel result={null} loading />);
    expect(screen.getByTestId('predicate-bundles-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there are no bundles', () => {
    render(<PredicateBundlesPanel result={computePredicateBundles([])} />);
    expect(screen.getByText('No knowledge graph yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(<PredicateBundlesPanel result={null} error="graph unavailable" onRetry={onRetry} />);
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('renders metric tiles, the summary caption, and the ranked bundle list with predicate chips', () => {
    render(<PredicateBundlesPanel result={mixed} />);
    expect(screen.getByText('Distinct pairs')).toBeInTheDocument();
    expect(screen.getByText('Thick relationships')).toBeInTheDocument();
    expect(screen.getByText('Max thickness')).toBeInTheDocument();
    expect(screen.getByText('Top bundles')).toBeInTheDocument();
    // 5 relations across 2 pairs -> fully-plural caption variant.
    expect(screen.getByText('5 relations across 2 pairs')).toBeInTheDocument();
    // thick pair's predicate chips render alphabetically. 'knows' appears in
    // both bundles (A->B thick, C->D thin) so use getAllByText for it.
    expect(screen.getAllByText('knows')).toHaveLength(2);
    expect(screen.getByText('trusts')).toBeInTheDocument();
    expect(screen.getByText('mentors')).toBeInTheDocument();
    // thin pair's totalRelations count badge "2×" appears.
    expect(screen.getByText('2×')).toBeInTheDocument();
  });

  it('uses an all-singular caption when there is exactly 1 relation in 1 pair', () => {
    const single = computePredicateBundles([rel('A', 'knows', 'B')]);
    render(<PredicateBundlesPanel result={single} />);
    expect(screen.getByText('1 relation across 1 pair')).toBeInTheDocument();
  });
});
