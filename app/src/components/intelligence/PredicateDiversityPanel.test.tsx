import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { computePredicateDiversity } from '../../lib/memory/predicateDiversity';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import PredicateDiversityPanel from './PredicateDiversityPanel';

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

// 4 relations: 3 'knows' + 1 'trusts' -> entropy ~0.811 bits, evenness ~81%.
const unbalanced = computePredicateDiversity([
  rel('A', 'knows', 'B'),
  rel('A', 'knows', 'C'),
  rel('A', 'knows', 'D'),
  rel('A', 'trusts', 'E'),
]);

describe('<PredicateDiversityPanel />', () => {
  it('renders the loading skeleton', () => {
    render(<PredicateDiversityPanel result={null} loading />);
    expect(screen.getByTestId('predicate-diversity-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there are no relations', () => {
    render(<PredicateDiversityPanel result={computePredicateDiversity([])} />);
    expect(screen.getByText('No knowledge graph yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(<PredicateDiversityPanel result={null} error="graph unavailable" onRetry={onRetry} />);
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('renders metric tiles, the summary caption, and the ranked frequency table', () => {
    render(<PredicateDiversityPanel result={unbalanced} />);
    expect(screen.getByText('Distinct predicates')).toBeInTheDocument();
    expect(screen.getByText('Entropy (bits)')).toBeInTheDocument();
    expect(screen.getByText('Evenness')).toBeInTheDocument();
    // entropy 0.81127… renders as "0.81".
    expect(screen.getByText('0.81')).toBeInTheDocument();
    // evenness 0.81127… -> rounded percent = 81%.
    expect(screen.getByText('81%')).toBeInTheDocument();
    expect(screen.getByText('4 relations counted')).toBeInTheDocument();
    expect(screen.getByText('Top predicates')).toBeInTheDocument();
    // ranking lists 'knows' and 'trusts'.
    expect(screen.getByText('knows')).toBeInTheDocument();
    expect(screen.getByText('trusts')).toBeInTheDocument();
    // frequency values render to one decimal place.
    expect(screen.getByText('75.0%')).toBeInTheDocument();
    expect(screen.getByText('25.0%')).toBeInTheDocument();
  });

  it('uses the singular caption when only one relation has been recorded', () => {
    const single = computePredicateDiversity([rel('A', 'knows', 'B')]);
    render(<PredicateDiversityPanel result={single} />);
    expect(screen.getByText('1 relation counted')).toBeInTheDocument();
  });
});
