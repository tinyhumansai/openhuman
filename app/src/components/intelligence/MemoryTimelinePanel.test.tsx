import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { computeTimeline } from '../../lib/memory/memoryTimeline';
import type { GraphRelation } from '../../utils/tauriCommands/memory';
import MemoryTimelinePanel from './MemoryTimelinePanel';

const NOW = 1_700_000_000;

function utc(year: number, month: number, day = 1): number {
  return Math.floor(Date.UTC(year, month - 1, day) / 1000);
}

function rel(updatedAt: number): GraphRelation {
  return {
    namespace: 'n',
    subject: 'You',
    predicate: 'p',
    object: 'x',
    attrs: {},
    updatedAt,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

const report = computeTimeline(
  [rel(utc(2023, 1, 10)), rel(utc(2023, 1, 20)), rel(utc(2023, 3, 5))],
  NOW
);

describe('<MemoryTimelinePanel />', () => {
  it('renders the loading skeleton', () => {
    render(<MemoryTimelinePanel report={null} loading />);
    expect(screen.getByTestId('memory-timeline-loading')).toBeInTheDocument();
  });

  it('renders the empty state when there are no facts', () => {
    render(<MemoryTimelinePanel report={computeTimeline([], NOW)} />);
    expect(screen.getByText('No knowledge graph yet.')).toBeInTheDocument();
  });

  it('renders an error with a working retry button', () => {
    const onRetry = vi.fn();
    render(<MemoryTimelinePanel report={null} error="graph unavailable" onRetry={onRetry} />);
    expect(screen.getByRole('alert').textContent).toMatch(/graph unavailable/);
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('renders summary tiles, the busiest caption, and per-month bars', () => {
    render(<MemoryTimelinePanel report={report} />);
    expect(screen.getByText('Facts')).toBeInTheDocument();
    expect(screen.getByText('Active months')).toBeInTheDocument();
    expect(screen.getByText('Last 30 days')).toBeInTheDocument();
    expect(screen.getByText('Facts learned per month')).toBeInTheDocument();
    expect(screen.getByText('2023-01')).toBeInTheDocument();
    expect(screen.getByText('2023-03')).toBeInTheDocument();
    expect(screen.getByText('Busiest: 2023-01 (2)')).toBeInTheDocument();
  });

  it('notes undated facts when present', () => {
    const withUndated = computeTimeline([rel(utc(2023, 5, 1)), rel(0), rel(0)], NOW);
    render(<MemoryTimelinePanel report={withUndated} />);
    expect(screen.getByText('2 fact(s) have no recorded date.')).toBeInTheDocument();
  });
});
