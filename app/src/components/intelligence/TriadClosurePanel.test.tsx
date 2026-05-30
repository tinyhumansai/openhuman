import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { computeTriadClosure } from '../../lib/memory/triadClosure';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import TriadClosurePanel from './TriadClosurePanel';

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

// Two intermediaries for (A, C) -> support=2, score = 2 / log(3).
const populated = computeTriadClosure([rel('A', 'B'), rel('B', 'C'), rel('A', 'D'), rel('D', 'C')]);

describe('<TriadClosurePanel />', () => {
  it('renders the loading skeleton', () => {
    render(<TriadClosurePanel result={null} loading />);
    expect(screen.getByTestId('triad-closure-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there are no nodes', () => {
    render(<TriadClosurePanel result={computeTriadClosure([])} />);
    expect(screen.getByText('No knowledge graph yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(<TriadClosurePanel result={null} error="graph unavailable" onRetry={onRetry} />);
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('renders metric tiles, the summary caption, and the suggested-edge worklist with intermediaries', () => {
    render(<TriadClosurePanel result={populated} />);
    expect(screen.getByText('Suggested edges')).toBeInTheDocument();
    expect(screen.getByText('Candidate pairs')).toBeInTheDocument();
    expect(screen.getByText('Minimum support')).toBeInTheDocument();
    expect(screen.getByText('Suggested edges to consider')).toBeInTheDocument();
    // Subject A and object C appear as the suggested edge.
    expect(screen.getByText('A')).toBeInTheDocument();
    expect(screen.getByText('C')).toBeInTheDocument();
    // Intermediary chips B and D render alphabetically.
    expect(screen.getByText('B')).toBeInTheDocument();
    expect(screen.getByText('D')).toBeInTheDocument();
    // Score 2 / log(3) ≈ 1.820 rounds to 3dp as "1.820".
    expect(screen.getByText('1.820')).toBeInTheDocument();
  });

  it('shows the all-filtered caption when every candidate is below minSupport', () => {
    // Single intermediary -> support=1 < default minSupport=2 -> all filtered.
    const filtered = computeTriadClosure([rel('A', 'B'), rel('B', 'C')]);
    render(<TriadClosurePanel result={filtered} />);
    expect(screen.getByText(/1 candidate pairs filtered out by support floor/)).toBeInTheDocument();
  });

  it('shows the no-candidates caption when the graph has no open wedges', () => {
    // Single edge -> no wedge possible.
    const flat = computeTriadClosure([rel('A', 'B')]);
    render(<TriadClosurePanel result={flat} />);
    expect(
      screen.getByText('No open triads — the graph has no wedges to close.')
    ).toBeInTheDocument();
  });
});
