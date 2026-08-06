import { render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it } from 'vitest';

import { store } from '../../../store';
import type { ProcessingTranscriptItem, ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import { PastTurnInsights } from './PastTurnInsights';

function renderInStore(ui: React.ReactNode) {
  return render(<Provider store={store}>{ui}</Provider>);
}

describe('PastTurnInsights', () => {
  it('replays a restored turn as interleaved thoughts + tool rows, not just tool cards', () => {
    // A reopened past turn carries both its reasoning trail and its tools.
    const entries: ToolTimelineEntry[] = [
      { id: 'c1', name: 'read_file', round: 0, seq: 0, status: 'success' },
    ];
    const transcript: ProcessingTranscriptItem[] = [
      { kind: 'thinking', round: 0, seq: 1, text: 'planning the search' },
      { kind: 'toolCall', round: 0, seq: 2, callId: 'c1' },
    ];

    renderInStore(<PastTurnInsights entries={entries} transcript={transcript} />);

    // The hidden reasoning replays (fix 1) — a restored turn is no longer
    // tool-cards-only.
    expect(screen.getByTestId('processing-thinking').textContent).toContain('planning the search');
    // And its tool step still renders in the interleaved view.
    expect(screen.getAllByTestId('processing-tool-row').length).toBeGreaterThan(0);
  });

  it('renders restored sub-agent transcripts beneath the trail', () => {
    const entries: ToolTimelineEntry[] = [
      {
        id: 'subagent:task-y',
        name: 'subagent:researcher',
        round: 0,
        seq: 0,
        status: 'success',
        subagent: {
          taskId: 'task-y',
          agentId: 'researcher',
          toolCalls: [],
          transcript: [{ kind: 'thinking', iteration: 1, text: 'child reasoning trail' }],
        },
      },
    ];
    const transcript: ProcessingTranscriptItem[] = [
      { kind: 'narration', round: 0, seq: 0, text: 'delegating to a researcher' },
    ];

    renderInStore(<PastTurnInsights entries={entries} transcript={transcript} />);

    const subagents = screen.getByTestId('past-turn-subagents');
    expect(subagents.textContent).toContain('child reasoning trail');
  });

  it('falls back to the tool-only timeline for a legacy turn with no transcript', () => {
    const entries: ToolTimelineEntry[] = [
      { id: 'c1', name: 'read_file', round: 0, seq: 0, status: 'success' },
    ];

    renderInStore(<PastTurnInsights entries={entries} transcript={[]} />);

    // The interleaved transcript view is absent; the tool-only block renders.
    expect(screen.queryByTestId('processing-transcript')).toBeNull();
    expect(screen.getByTestId('agent-task-insights')).toBeTruthy();
  });
});
