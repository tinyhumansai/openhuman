import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { computeNamespaceOverview } from '../../lib/memory/namespaceOverview';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import NamespaceOverviewPanel from './NamespaceOverviewPanel';

function rel(namespace: string | null, subject: string, object: string): GraphRelation {
  return {
    namespace,
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

const report = computeNamespaceOverview([
  rel('work', 'A', 'B'),
  rel('work', 'B', 'C'),
  rel(null, 'P', 'Q'),
]);

describe('<NamespaceOverviewPanel />', () => {
  it('renders the loading skeleton', () => {
    render(<NamespaceOverviewPanel report={null} loading />);
    expect(screen.getByTestId('namespace-overview-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there are no namespaces', () => {
    render(<NamespaceOverviewPanel report={computeNamespaceOverview([])} />);
    expect(screen.getByText('No knowledge graph yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(<NamespaceOverviewPanel report={null} error="graph unavailable" onRetry={onRetry} />);
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('renders summary tiles and the per-namespace list (un-namespaced labeled)', () => {
    render(<NamespaceOverviewPanel report={report} />);
    expect(screen.getByText('Namespaces')).toBeInTheDocument();
    expect(screen.getByText('Facts')).toBeInTheDocument();
    expect(screen.getByText('By namespace')).toBeInTheDocument();
    expect(screen.getByText('work')).toBeInTheDocument();
    // the null namespace renders with the "un-namespaced" label
    expect(screen.getByText('(un-namespaced)')).toBeInTheDocument();
  });
});
