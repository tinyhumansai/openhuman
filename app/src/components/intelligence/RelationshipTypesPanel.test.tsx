import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { computeRelationshipTypes } from '../../lib/memory/relationshipTypes';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import RelationshipTypesPanel from './RelationshipTypesPanel';

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

const report = computeRelationshipTypes([
  rel('A', 'knows', 'B'),
  rel('B', 'knows', 'A'),
  rel('A', 'likes', 'C'),
]);

describe('<RelationshipTypesPanel />', () => {
  it('renders the loading skeleton', () => {
    render(<RelationshipTypesPanel report={null} loading />);
    expect(screen.getByTestId('relationship-types-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there are no edges', () => {
    render(<RelationshipTypesPanel report={computeRelationshipTypes([])} />);
    expect(screen.getByText('No knowledge graph yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(<RelationshipTypesPanel report={null} error="graph unavailable" onRetry={onRetry} />);
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('renders metric tiles and the ranked predicate list', () => {
    render(<RelationshipTypesPanel report={report} />);
    expect(screen.getByText('Edges')).toBeInTheDocument();
    expect(screen.getByText('Predicates')).toBeInTheDocument();
    expect(screen.getByText('Reciprocity')).toBeInTheDocument();
    expect(screen.getByText('Most-used relationships')).toBeInTheDocument();
    expect(screen.getByText('knows')).toBeInTheDocument();
    expect(screen.getByText('likes')).toBeInTheDocument();
  });
});
