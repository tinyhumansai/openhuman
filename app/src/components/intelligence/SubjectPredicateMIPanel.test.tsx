import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { computeSubjectPredicateMI } from '../../lib/memory/subjectPredicateMI';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import SubjectPredicateMIPanel from './SubjectPredicateMIPanel';

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

// s1 fully specialised (all "knows"), s2 fully generalist (3 distinct predicates evenly).
const profile = computeSubjectPredicateMI([
  rel('s1', 'knows', 'a'),
  rel('s1', 'knows', 'b'),
  rel('s1', 'knows', 'c'),
  rel('s2', 'knows', 'a'),
  rel('s2', 'trusts', 'b'),
  rel('s2', 'mentors', 'c'),
]);

describe('<SubjectPredicateMIPanel />', () => {
  it('renders the loading skeleton', () => {
    render(<SubjectPredicateMIPanel result={null} loading />);
    expect(screen.getByTestId('subject-predicate-mi-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there are no relations', () => {
    render(<SubjectPredicateMIPanel result={computeSubjectPredicateMI([])} />);
    expect(screen.getByText('No knowledge graph yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(<SubjectPredicateMIPanel result={null} error="graph unavailable" onRetry={onRetry} />);
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('renders metric tiles, summary caption, and the per-subject specialisation table', () => {
    render(<SubjectPredicateMIPanel result={profile} />);
    expect(screen.getByText('Mutual information (bits)')).toBeInTheDocument();
    expect(screen.getByText('Normalised MI')).toBeInTheDocument();
    expect(screen.getByText('Subjects')).toBeInTheDocument();
    expect(screen.getByText('Most-to-least specialised subjects')).toBeInTheDocument();
    // specialist s1 leads with 100% specialisation; generalist s2 has 0%.
    expect(screen.getByText('s1')).toBeInTheDocument();
    expect(screen.getByText('s2')).toBeInTheDocument();
    expect(screen.getByText('100%')).toBeInTheDocument();
    expect(screen.getByText('0%')).toBeInTheDocument();
  });
});
