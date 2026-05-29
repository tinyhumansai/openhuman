import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { computeGraphCore } from '../../lib/memory/graphCore';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import GraphCorePanel from './GraphCorePanel';

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

// Triangle A-B-C (2-core) plus pendant D off A -> degeneracy 2, shells {2:3,1:1}.
const cored = computeGraphCore([rel('A', 'B'), rel('B', 'C'), rel('C', 'A'), rel('A', 'D')]);

describe('<GraphCorePanel />', () => {
  it('renders the loading skeleton', () => {
    render(<GraphCorePanel result={null} loading />);
    expect(screen.getByTestId('graph-core-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there are no nodes', () => {
    render(<GraphCorePanel result={computeGraphCore([])} />);
    expect(screen.getByText('No knowledge graph yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(<GraphCorePanel result={null} error="graph unavailable" onRetry={onRetry} />);
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('renders metric tiles, the shell decomposition, and the ranked table', () => {
    render(<GraphCorePanel result={cored} />);
    expect(screen.getByText('Entities')).toBeInTheDocument();
    expect(screen.getByText('Connections')).toBeInTheDocument();
    expect(screen.getByText('Degeneracy')).toBeInTheDocument();
    expect(screen.getByText('Shell decomposition')).toBeInTheDocument();
    expect(screen.getByText('Deepest-core entities')).toBeInTheDocument();
    // shell labels for the two coreness levels present.
    expect(screen.getByText('2-core')).toBeInTheDocument();
    expect(screen.getByText('1-core')).toBeInTheDocument();
    // densest shell holds the triangle (3 entities at the 2-core).
    expect(screen.getByText(/2-core · 3 entities/)).toBeInTheDocument();
  });

  it('badges the deepest-core members and not the periphery', () => {
    render(<GraphCorePanel result={cored} />);
    // three triangle members carry the core badge; the pendant D does not.
    expect(screen.getAllByText('core')).toHaveLength(3);
  });
});
