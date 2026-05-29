import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { computeDocumentProvenance } from '../../lib/memory/documentProvenance';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import DocumentProvenancePanel from './DocumentProvenancePanel';

function rel(
  subject: string,
  predicate: string,
  object: string,
  documentIds: string[]
): GraphRelation {
  return {
    namespace: 'n',
    subject,
    predicate,
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount: 1,
    orderIndex: null,
    documentIds,
    chunkIds: [],
  };
}

// d2 carries 2 relations (largest); d1 corroborates one of them; d3 contributes
// an exclusive disjoint pair. Pair (d1, d2) has Jaccard 2/3.
const mixed = computeDocumentProvenance([
  rel('A', 'knows', 'B', ['d1', 'd2']),
  rel('B', 'knows', 'C', ['d2']),
  rel('D', 'knows', 'E', ['d3']),
]);

describe('<DocumentProvenancePanel />', () => {
  it('renders the loading skeleton', () => {
    render(<DocumentProvenancePanel result={null} loading />);
    expect(screen.getByTestId('document-provenance-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there are no documents and no unsourced relations', () => {
    render(<DocumentProvenancePanel result={computeDocumentProvenance([])} />);
    expect(screen.getByText('No knowledge graph yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(<DocumentProvenancePanel result={null} error="graph unavailable" onRetry={onRetry} />);
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('renders metric tiles, the docs table with load-bearing badges, and the overlap pair list', () => {
    render(<DocumentProvenancePanel result={mixed} />);
    expect(screen.getByText('Documents')).toBeInTheDocument();
    expect(screen.getByText('Provenance coverage')).toBeInTheDocument();
    expect(screen.getByText('Single-source share')).toBeInTheDocument();
    expect(screen.getByText('Per-document footprint')).toBeInTheDocument();
    expect(screen.getByText('Top overlap pairs')).toBeInTheDocument();
    // d2 and d3 each have an exclusive relation -> 2 load-bearing badges.
    expect(screen.getAllByText('load-bearing')).toHaveLength(2);
    // unsourced caption with the zero count branch.
    expect(screen.getByText('0 relations without provenance')).toBeInTheDocument();
    // single overlap pair (d1, d2) — Jaccard 2/3 -> rounded "67%". The same
    // "67%" also appears as the Single-source-share tile value (2 of 3 sourced
    // relations are exclusive), so use getAllByText.
    expect(screen.getAllByText('67%')).toHaveLength(2);
    expect(screen.getByText('2/3')).toBeInTheDocument();
  });

  it('uses the singular caption when exactly one relation lacks provenance', () => {
    const single = computeDocumentProvenance([rel('A', 'p', 'B', [])]);
    render(<DocumentProvenancePanel result={single} />);
    expect(screen.getByText('1 relation without provenance')).toBeInTheDocument();
  });
});
