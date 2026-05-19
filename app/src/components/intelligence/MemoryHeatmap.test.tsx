import { fireEvent, render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { MemoryHeatmap } from './MemoryHeatmap';

function todaySeconds(): number {
  return Math.floor(Date.now() / 1000);
}

/**
 * MemoryHeatmap's display window ends at midnight-start-of-today. Timestamps
 * later than midnight today are placed on a future cell and NOT counted in
 * the total/peak summary. Use yesterday's epoch-seconds for "should be
 * counted" assertions.
 */
function yesterdaySeconds(): number {
  return Math.floor((Date.now() - 24 * 60 * 60 * 1000) / 1000);
}

describe('<MemoryHeatmap />', () => {
  it('renders a skeleton when loading', () => {
    const { container } = render(<MemoryHeatmap timestamps={[]} loading />);
    expect(container.querySelector('.animate-pulse')).not.toBeNull();
    // SVG only renders in the non-loading branch.
    expect(container.querySelector('svg')).toBeNull();
  });

  it('renders the SVG grid when not loading', () => {
    const { container } = render(<MemoryHeatmap timestamps={[]} />);
    const svg = container.querySelector('svg');
    expect(svg).not.toBeNull();
    // Many rect cells, at least one per displayed day.
    expect(svg!.querySelectorAll('rect').length).toBeGreaterThan(30);
  });

  it('reports total counted events from timestamps inside the display window', () => {
    const y = yesterdaySeconds();
    // 4 events yesterday — all inside the 8-month window.
    const timestamps = [y, y - 30, y - 60, y - 90];
    render(<MemoryHeatmap timestamps={timestamps} />);
    // Total renders as separate text nodes: "<n> events over the last …".
    const summary = document.body.textContent ?? '';
    expect(summary).toMatch(/4 events/);
    // Peak (max daily count) is 4 because all four fall on the same day.
    expect(summary).toMatch(/Peak.*4/);
  });

  it('shows a hover tooltip when a cell is mouse-entered', () => {
    const now = todaySeconds();
    const { container } = render(<MemoryHeatmap timestamps={[now]} />);
    const rect = container.querySelector('rect');
    expect(rect).not.toBeNull();
    fireEvent.mouseEnter(rect!);
    // Tooltip is rendered as a fixed-positioned div with z-50.
    expect(container.querySelector('.fixed.z-50')).not.toBeNull();
    fireEvent.mouseLeave(rect!);
    expect(container.querySelector('.fixed.z-50')).toBeNull();
  });

  it('accepts millisecond-precision timestamps (>9999999999)', () => {
    // Yesterday in ms so it lands inside the counted window.
    const yMs = Date.now() - 24 * 60 * 60 * 1000;
    render(<MemoryHeatmap timestamps={[yMs]} />);
    const summary = document.body.textContent ?? '';
    expect(summary).toMatch(/1 event/);
  });
});
