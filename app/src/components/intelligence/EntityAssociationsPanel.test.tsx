import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { computeEntityAssociations } from '../../lib/memory/entityAssociations';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import EntityAssociationsPanel from './EntityAssociationsPanel';

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

// X,Y share neighbours A,B (inferred pair); the triangle A-B-? gives linked pairs.
const report = computeEntityAssociations([
  rel('X', 'A'),
  rel('X', 'B'),
  rel('Y', 'A'),
  rel('Y', 'B'),
]);

describe('<EntityAssociationsPanel />', () => {
  it('renders the loading skeleton', () => {
    render(<EntityAssociationsPanel report={null} loading />);
    expect(screen.getByTestId('entity-associations-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there are no pairs', () => {
    render(<EntityAssociationsPanel report={computeEntityAssociations([rel('A', 'B')])} />);
    expect(screen.getByText('No associations yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(<EntityAssociationsPanel report={null} error="graph unavailable" onRetry={onRetry} />);
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('renders metric tiles and ranked pairs with an inferred badge', () => {
    render(<EntityAssociationsPanel report={report} />);
    expect(screen.getByText('Entities')).toBeInTheDocument();
    expect(screen.getByText('Associations')).toBeInTheDocument();
    expect(screen.getByText('Strongest associations')).toBeInTheDocument();
    // X~Y and A~B are perfect, non-linked associations -> "inferred" badges.
    expect(screen.getAllByText('inferred').length).toBeGreaterThanOrEqual(1);
  });

  it('notes truncation when more pairs exist than are shown', () => {
    const leaves = ['a', 'b', 'c', 'd', 'e'];
    const big = computeEntityAssociations(
      leaves.map(l => rel(l, 'H')),
      { limit: 3 }
    );
    render(<EntityAssociationsPanel report={big} />);
    expect(screen.getByText('Showing 3 of 10 — strongest first.')).toBeInTheDocument();
  });
});
