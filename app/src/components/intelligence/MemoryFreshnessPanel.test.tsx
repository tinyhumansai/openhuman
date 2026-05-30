import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { computeFreshness } from '../../lib/memory/memoryFreshness';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import MemoryFreshnessPanel from './MemoryFreshnessPanel';

const NOW = 1_700_000_000;
const DAY = 86400;

function rel(subject: string, object: string, agoDays: number): GraphRelation {
  return {
    namespace: 'n',
    subject,
    predicate: 'likes',
    object,
    attrs: {},
    updatedAt: NOW - agoDays * DAY,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

const report = computeFreshness(
  [rel('You', 'Berlin', 0), rel('You', 'coffee', 30), rel('You', 'guitar', 90)],
  NOW
);

describe('<MemoryFreshnessPanel />', () => {
  it('renders the loading skeleton', () => {
    render(<MemoryFreshnessPanel report={null} loading />);
    expect(screen.getByTestId('memory-freshness-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there are no facts', () => {
    render(<MemoryFreshnessPanel report={computeFreshness([], NOW)} />);
    expect(screen.getByText('No knowledge graph yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(<MemoryFreshnessPanel report={null} error="graph unavailable" onRetry={onRetry} />);
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('renders status tiles and the re-confirm queue (non-fresh facts only)', () => {
    render(<MemoryFreshnessPanel report={report} />);
    expect(screen.getByText('Fresh')).toBeInTheDocument();
    expect(screen.getByText('Fading')).toBeInTheDocument();
    expect(screen.getByText('Stale')).toBeInTheDocument();
    expect(screen.getByText('Re-confirm queue')).toBeInTheDocument();
    // The stale 'guitar' fact and fading 'coffee' fact are in the queue...
    expect(screen.getByText(/guitar/)).toBeInTheDocument();
    expect(screen.getByText(/coffee/)).toBeInTheDocument();
    // ...but the fresh 'Berlin' fact is not.
    expect(screen.queryByText(/Berlin/)).not.toBeInTheDocument();
  });

  it('shows the all-fresh message when nothing needs re-confirming', () => {
    const allFresh = computeFreshness([rel('You', 'Berlin', 0)], NOW);
    render(<MemoryFreshnessPanel report={allFresh} />);
    expect(
      screen.getByText('Every fact is still fresh — nothing to re-confirm.')
    ).toBeInTheDocument();
  });

  it('notes when the re-confirm queue is truncated past the row cap', () => {
    // 60 stale facts -> the queue is capped at 50 and a "showing 50 of 60" note appears.
    const many = Array.from({ length: 60 }, (_, i) => rel('You', `fact${i}`, 365));
    render(<MemoryFreshnessPanel report={computeFreshness(many, NOW)} />);
    expect(screen.getByText('Showing 50 of 60 — address these first.')).toBeInTheDocument();
  });
});
