import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { ScoreBreakdown } from '../../../utils/tauriCommands';
import { MemoryChunkScoreBars } from '../MemoryChunkScoreBars';

describe('MemoryChunkScoreBars', () => {
  it('renders one row per signal with a clamped, formatted value', () => {
    const breakdown: ScoreBreakdown = {
      total: 0.65,
      threshold: 0.5,
      kept: true,
      llm_consulted: false,
      signals: [
        { name: 'recency', weight: 0.5, value: 0.83 },
        { name: 'salience', weight: 0.3, value: 0.4 },
        // Out-of-range and NaN both clamp to 0..1 — the bar must not crash
        // or render past the track.
        { name: 'pinned', weight: 0.1, value: 1.7 },
        { name: 'broken', weight: 0.1, value: Number.NaN },
      ],
    };
    render(<MemoryChunkScoreBars breakdown={breakdown} />);

    expect(screen.getByText('recency')).toBeInTheDocument();
    expect(screen.getByText('0.83')).toBeInTheDocument();
    expect(screen.getByText('0.40')).toBeInTheDocument();
    // Clamped to 1.00 (over-range) and 0.00 (NaN).
    expect(screen.getByText('1.00')).toBeInTheDocument();
    expect(screen.getByText('0.00')).toBeInTheDocument();

    // ARIA labels on the bars are how a screen reader would surface the
    // percentage; check the over-range one collapsed to "100 percent".
    expect(screen.getByLabelText('pinned score 100 percent')).toBeInTheDocument();
    expect(screen.getByLabelText('broken score 0 percent')).toBeInTheDocument();
  });

  it('shows the threshold footer with kept/dropped state', () => {
    const breakdown: ScoreBreakdown = {
      total: 0.2,
      threshold: 0.5,
      kept: false,
      llm_consulted: false,
      signals: [{ name: 'recency', weight: 1, value: 0.2 }],
    };
    render(<MemoryChunkScoreBars breakdown={breakdown} />);
    expect(screen.getByText(/dropped at 0\.50/)).toBeInTheDocument();
  });
});
