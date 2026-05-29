import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { computeEntityDuplicates } from '../../lib/memory/entityDuplicates';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import EntityDuplicatesPanel from './EntityDuplicatesPanel';

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

const dupReport = computeEntityDuplicates([
  rel('Alice', 'Bob'),
  rel('alice', 'Carol'),
  rel(' Alice ', 'Dave'),
]);

describe('<EntityDuplicatesPanel />', () => {
  it('renders the loading skeleton', () => {
    render(<EntityDuplicatesPanel report={null} loading />);
    expect(screen.getByTestId('entity-duplicates-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there is no graph', () => {
    render(<EntityDuplicatesPanel report={computeEntityDuplicates([])} />);
    expect(screen.getByText('No knowledge graph yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(<EntityDuplicatesPanel report={null} error="graph unavailable" onRetry={onRetry} />);
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('renders the all-clean message when entities exist but no duplicates', () => {
    const clean = computeEntityDuplicates([rel('Alice', 'Bob')]);
    render(<EntityDuplicatesPanel report={clean} />);
    expect(
      screen.getByText('No duplicate spellings detected — your entities look clean.')
    ).toBeInTheDocument();
  });

  it('renders duplicate clusters with their variants', () => {
    render(<EntityDuplicatesPanel report={dupReport} />);
    expect(screen.getByText('Entities')).toBeInTheDocument();
    expect(screen.getByText('Duplicate sets')).toBeInTheDocument();
    expect(screen.getByText('Likely duplicate entities')).toBeInTheDocument();
    // All three spelling variants render. 'Alice' and ' Alice ' both normalize
    // to the same visible text under Testing Library, so there are two of them.
    expect(screen.getByText('alice')).toBeInTheDocument();
    expect(screen.getAllByText('Alice')).toHaveLength(2);
  });
});
