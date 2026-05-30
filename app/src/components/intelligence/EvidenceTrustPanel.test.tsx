import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { computeEvidenceTrust } from '../../lib/memory/evidenceTrust';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import EvidenceTrustPanel from './EvidenceTrustPanel';

function rel(
  subject: string,
  predicate: string,
  object: string,
  evidenceCount: number
): GraphRelation {
  return {
    namespace: 'n',
    subject,
    predicate,
    object,
    attrs: {},
    updatedAt: 0,
    evidenceCount,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

// Mix: prolific (4 thin facts) vs quiet (1 heavily-corroborated fact) +
// an under-corroborated edge so the worklist is non-empty.
const mixed = computeEvidenceTrust([
  rel('prolific', 'knows', 'a', 1),
  rel('prolific', 'knows', 'b', 1),
  rel('prolific', 'knows', 'c', 1),
  rel('prolific', 'knows', 'd', 1),
  rel('quiet', 'recommends', 'rare', 20),
]);

describe('<EvidenceTrustPanel />', () => {
  it('renders the loading skeleton', () => {
    render(<EvidenceTrustPanel result={null} loading />);
    expect(screen.getByTestId('evidence-trust-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there are no relations', () => {
    render(<EvidenceTrustPanel result={computeEvidenceTrust([])} />);
    expect(screen.getByText('No knowledge graph yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(<EvidenceTrustPanel result={null} error="graph unavailable" onRetry={onRetry} />);
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('shows the degraded banner when every relation has evidence === 1', () => {
    // All evidence===1 -> degraded path.
    const degraded = computeEvidenceTrust([rel('A', 'p', 'B', 1), rel('B', 'p', 'C', 1)]);
    render(<EvidenceTrustPanel result={degraded} />);
    expect(
      screen.getByText('Evidence signal sparse — populate evidenceCount to unlock this lens.')
    ).toBeInTheDocument();
  });

  it('renders metric tiles, per-entity trust ranking, and predicate reliability', () => {
    render(<EvidenceTrustPanel result={mixed} />);
    expect(screen.getByText('Evidence Gini')).toBeInTheDocument();
    expect(screen.getByText('Entities weighted')).toBeInTheDocument();
    expect(screen.getByText('Total evidence')).toBeInTheDocument();
    expect(screen.getByText('Trust Quotient ranking')).toBeInTheDocument();
    expect(screen.getByText('Predicate Reliability Index')).toBeInTheDocument();
    // quiet has TQ 20, much higher than prolific's TQ 1.
    expect(screen.getByText('quiet')).toBeInTheDocument();
    expect(screen.getByText('prolific')).toBeInTheDocument();
    // For this fixture (positives [1,1,1,1,20], median=1, threshold=1), no
    // relation falls below threshold -> the "no worklist" caption renders.
    expect(
      screen.getByText(
        'No under-corroborated relations — every assertion meets the evidence threshold.'
      )
    ).toBeInTheDocument();
  });

  it('renders an under-corroborated worklist when threshold catches relations', () => {
    // Positive evidences [1, 1, 8, 8, 8, 8] -> median index 2 -> 8;
    // threshold = max(1, floor(8/4)) = 2. Two evidence=1 entries flagged.
    const worklist = computeEvidenceTrust([
      rel('S', 'p', 'X', 1),
      rel('S', 'p', 'Y', 1),
      rel('M', 'p', 'A', 8),
      rel('M', 'p', 'B', 8),
      rel('M', 'p', 'C', 8),
      rel('M', 'p', 'D', 8),
    ]);
    render(<EvidenceTrustPanel result={worklist} />);
    expect(screen.getByText('Under-corroborated worklist')).toBeInTheDocument();
    expect(screen.getAllByText('ev 1')).toHaveLength(2);
  });
});
