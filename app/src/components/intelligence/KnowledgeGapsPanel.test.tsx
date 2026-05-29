import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { computeKnowledgeGaps } from '../../lib/memory/knowledgeGaps';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import KnowledgeGapsPanel from './KnowledgeGapsPanel';

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

const report = computeKnowledgeGaps([rel('A', 'B'), rel('B', 'C'), rel('D', 'D')]);

describe('<KnowledgeGapsPanel />', () => {
  it('renders the loading skeleton', () => {
    render(<KnowledgeGapsPanel report={null} loading />);
    expect(screen.getByTestId('knowledge-gaps-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there is no graph', () => {
    render(<KnowledgeGapsPanel report={computeKnowledgeGaps([])} />);
    expect(screen.getByText('No knowledge graph yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(<KnowledgeGapsPanel report={null} error="graph unavailable" onRetry={onRetry} />);
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('renders summary tiles and the gap list with kind badges', () => {
    render(<KnowledgeGapsPanel report={report} />);
    expect(screen.getByText('Entities')).toBeInTheDocument();
    expect(screen.getByText('Orphans')).toBeInTheDocument();
    expect(screen.getByText('Leaves')).toBeInTheDocument();
    expect(screen.getByText('Sparse entities')).toBeInTheDocument();
    expect(screen.getByText('D')).toBeInTheDocument(); // orphan
    expect(screen.getByText('orphan')).toBeInTheDocument();
    expect(screen.getAllByText('leaf').length).toBeGreaterThanOrEqual(1);
  });

  it('renders the all-connected message when there are no gaps', () => {
    const clean = computeKnowledgeGaps([rel('A', 'B'), rel('B', 'C'), rel('C', 'A')]);
    render(<KnowledgeGapsPanel report={clean} />);
    expect(screen.getByText('Every entity is well-connected — no gaps found.')).toBeInTheDocument();
  });
});
