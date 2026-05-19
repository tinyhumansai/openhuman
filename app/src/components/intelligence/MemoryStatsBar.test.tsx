import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { MemoryStatsBar } from './MemoryStatsBar';

const BASE_PROPS = {
  totalDocs: 1234,
  totalFiles: 7,
  totalNamespaces: 3,
  totalRelations: 42,
  totalSessions: 11,
  totalTokens: 9876,
  estimatedStorageBytes: 2 * 1024 * 1024, // 2 MB
  oldestDocTimestamp: null,
  newestDocTimestamp: null,
  docsToday: 0,
};

describe('<MemoryStatsBar />', () => {
  it('formats large numeric values with thousands separators', () => {
    render(<MemoryStatsBar {...BASE_PROPS} />);
    // totalDocs => 1,234
    expect(screen.getByText('1,234')).toBeInTheDocument();
    // totalTokens => 9,876 lives in a sub-label
    expect(screen.getByText(/9,876/)).toBeInTheDocument();
  });

  it('formats the storage cell in human-readable bytes', () => {
    // formatBytes uses .toFixed(1) for values < 10 ("2.0 MB"), Math.round otherwise.
    render(<MemoryStatsBar {...BASE_PROPS} />);
    expect(screen.getByText('2.0 MB')).toBeInTheDocument();
  });

  it('rounds storage values >= 10 of a unit', () => {
    render(<MemoryStatsBar {...BASE_PROPS} estimatedStorageBytes={42 * 1024 * 1024} />);
    expect(screen.getByText('42 MB')).toBeInTheDocument();
  });

  it('renders "--" placeholders when the corresponding fields are null/zero', () => {
    render(
      <MemoryStatsBar
        {...BASE_PROPS}
        estimatedStorageBytes={0}
        totalSessions={null}
        totalTokens={null}
      />
    );
    // storage and sessions both fall back to '--'
    const placeholders = screen.getAllByText('--');
    expect(placeholders.length).toBeGreaterThanOrEqual(2);
  });

  it('renders the docs-today sub-label only when docsToday > 0', () => {
    const { rerender } = render(<MemoryStatsBar {...BASE_PROPS} docsToday={5} />);
    expect(screen.getByText(/\+5/)).toBeInTheDocument();
    rerender(<MemoryStatsBar {...BASE_PROPS} docsToday={0} />);
    expect(screen.queryByText(/^\+/)).not.toBeInTheDocument();
  });

  it('renders a relative time-ago when oldest/newest timestamps are provided', () => {
    // Freeze the clock so the relative offsets resolve to exactly "5h ago"
    // and "10m ago" regardless of when the suite runs.
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-03-04T12:00:00.000Z'));
    try {
      const now = Math.floor(Date.now() / 1000);
      render(
        <MemoryStatsBar
          {...BASE_PROPS}
          oldestDocTimestamp={now - 3600 * 5} // 5h ago
          newestDocTimestamp={now - 60 * 10} // 10m ago
        />
      );
      expect(screen.getByText('5h ago')).toBeInTheDocument();
      expect(screen.getByText(/10m ago/)).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it('shows a skeleton placeholder when loading', () => {
    const { container } = render(<MemoryStatsBar {...BASE_PROPS} loading />);
    // 6 stat tiles, each renders an animate-pulse skeleton instead of value.
    expect(container.querySelectorAll('.animate-pulse').length).toBe(6);
  });
});
