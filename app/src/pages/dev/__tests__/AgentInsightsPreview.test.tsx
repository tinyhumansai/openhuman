/**
 * `AgentInsightsPreview` is the dev-only harness behind `#/dev/agent-insights`.
 * It had no tests, and being unlinked from any nav is exactly why: nothing
 * notices when it breaks. It is a fixture consumer, so it breaks whenever
 * `ToolTimelineEntry` changes shape — which is the regression worth catching,
 * because the next person to touch that type will not open this page.
 *
 * `ToolTimelineBlock` and `AgentProcessSourcePanel` are mocked so this asserts
 * the harness's own two jobs: the settled-entry derivation it computes, and the
 * panel open/close wiring. Their rendering is their own tests' business.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import AgentInsightsPreview from '../AgentInsightsPreview';

const timelineProps: { entries: ToolTimelineEntry[]; onViewSubagent: () => void }[] = [];
const panelProps: { open: boolean; entries: ToolTimelineEntry[]; onClose: () => void }[] = [];

vi.mock('../../../features/conversations/components/ToolTimelineBlock', () => ({
  ToolTimelineBlock: (props: { entries: ToolTimelineEntry[]; onViewSubagent: () => void }) => {
    timelineProps.push(props);
    return (
      <button data-testid="timeline" onClick={props.onViewSubagent}>
        timeline({props.entries.length})
      </button>
    );
  },
}));

vi.mock('../../../features/conversations/components/AgentProcessSourcePanel', () => ({
  AgentProcessSourcePanel: (props: {
    open: boolean;
    entries: ToolTimelineEntry[];
    onClose: () => void;
  }) => {
    panelProps.push(props);
    return props.open ? (
      <div data-testid="source-panel">
        <button onClick={props.onClose}>close</button>
      </div>
    ) : null;
  },
}));

const renderPreview = () => {
  timelineProps.length = 0;
  panelProps.length = 0;
  render(<AgentInsightsPreview />);
  // Section order is Running, then Settled.
  return { running: timelineProps[0].entries, settled: timelineProps[1].entries };
};

describe('AgentInsightsPreview', () => {
  it('renders both timeline sections and the source-panel trigger', () => {
    renderPreview();

    expect(screen.getAllByTestId('timeline')).toHaveLength(2);
    expect(
      screen.getByRole('button', { name: 'View full agent process Source →' })
    ).toBeInTheDocument();
  });

  it('feeds the same fixture shape to both sections', () => {
    const { running, settled } = renderPreview();

    expect(running.length).toBeGreaterThan(0);
    expect(settled).toHaveLength(running.length);
    expect(settled.map(e => e.id)).toEqual(running.map(e => e.id));
  });

  it('exercises every row variant in the running fixture', () => {
    const { running } = renderPreview();

    // The harness exists to eyeball these together; if a fixture stops covering
    // one, the page silently stops previewing that state.
    expect(running.some(e => e.status === 'running')).toBe(true);
    expect(running.some(e => e.status === 'success')).toBe(true);
    expect(running.some(e => e.subagent)).toBe(true);
    expect(running.some(e => e.status === 'error')).toBe(true);
  });

  it('settles every non-error entry to success and leaves errors alone', () => {
    const { running, settled } = renderPreview();

    // Precondition, not decoration: the "leaves errors alone" half of this test
    // only exercises anything if the fixture actually contains an error entry.
    // Without this, removing the error fixture (or flipping it to 'success')
    // would leave the test green while it silently stopped checking
    // preservation at all.
    expect(running.some(e => e.status === 'error')).toBe(true);

    settled.forEach((entry, i) => {
      expect(entry.status).toBe(running[i].status === 'error' ? 'error' : 'success');
    });
    expect(settled.some(e => e.status === 'error')).toBe(true);
    expect(settled.some(e => e.status === 'running')).toBe(false);
  });

  it('swaps a subagent in-progress iteration for a final count and elapsed time', () => {
    const { settled } = renderPreview();
    const sub = settled.find(e => e.subagent)?.subagent;

    expect(sub).toBeDefined();
    // `childIteration` is the live "on step N" counter; a settled run must not
    // still be advertising one.
    expect(sub?.childIteration).toBeUndefined();
    expect(sub?.iterations).toBe(6);
    expect(sub?.elapsedMs).toBe(49200);
    expect(sub?.toolCalls.every(c => c.status === 'success')).toBe(true);
    expect(sub?.toolCalls.every(c => c.elapsedMs === 2600)).toBe(true);
  });

  it('opens the source panel from the explicit button and closes it again', () => {
    renderPreview();

    expect(screen.queryByTestId('source-panel')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'View full agent process Source →' }));
    expect(screen.getByTestId('source-panel')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'close' }));
    expect(screen.queryByTestId('source-panel')).not.toBeInTheDocument();
  });

  it('also opens the panel from a timeline sub-agent click', () => {
    renderPreview();

    fireEvent.click(screen.getAllByTestId('timeline')[0]);
    expect(screen.getByTestId('source-panel')).toBeInTheDocument();
  });

  it('hands the panel the settled entries, not the running ones', () => {
    renderPreview();

    const latest = panelProps[panelProps.length - 1];
    expect(latest.entries.some(e => e.status === 'running')).toBe(false);
  });
});
