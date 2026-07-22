import { render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it } from 'vitest';

import { store } from '../../../store';
import type { DerivedDisplayItem } from '../../../types/derivedTranscript';
import { PastTurnInsights } from '../components/PastTurnInsights';
import { mapDisplayItems } from './mapDisplayItems';

function renderInStore(ui: React.ReactNode) {
  return render(<Provider store={store}>{ui}</Provider>);
}

/** Newest-first page from chronological items (as the RPC returns). */
function newestFirst(chronological: DerivedDisplayItem[]): DerivedDisplayItem[] {
  return [...chronological].reverse();
}

describe('derived transcript restore (mapper → PastTurnInsights)', () => {
  it('renders a restored turn with reasoning, tool rows, and a sub-agent trail', () => {
    // A settled turn's projected display items, exactly as `threads_transcript_get`
    // returns them (newest-first). This is a NON-newest turn so the mapper keeps it.
    const chronological: DerivedDisplayItem[] = [
      { kind: 'turnBoundary', requestId: 'req-1' },
      { kind: 'userMessage', content: 'research this', requestId: 'req-1' },
      { kind: 'reasoning', text: 'planning the research' },
      { kind: 'assistantMessage', content: 'searching now', interim: true, requestId: 'req-1' },
      {
        kind: 'toolCall',
        callId: 'c1',
        name: 'read_file',
        args: { path: 'notes.md' },
        result: 'ok',
        status: 'success',
      },
      {
        kind: 'subagent',
        id: 'researcher',
        items: [
          { kind: 'reasoning', text: 'child reasoning trail' },
          {
            kind: 'toolCall',
            callId: 'child-1',
            name: 'web_search',
            args: { q: 'topic' },
            result: 'hits',
            status: 'success',
          },
        ],
      },
      { kind: 'assistantMessage', content: 'the final answer', requestId: 'req-1' },
      // A later turn so req-1 is not the newest (which the mapper would skip).
      { kind: 'turnBoundary', requestId: 'req-2' },
      { kind: 'reasoning', text: 'newest turn thought' },
    ];

    const { timelines, transcripts } = mapDisplayItems(newestFirst(chronological));

    renderInStore(
      <PastTurnInsights entries={timelines['req-1']} transcript={transcripts['req-1']} />
    );

    // Reasoning replays.
    expect(screen.getByTestId('processing-thinking').textContent).toContain(
      'planning the research'
    );
    // Interim narration replays (not the final answer — that renders from the message).
    expect(screen.getByTestId('processing-transcript').textContent).toContain('searching now');
    expect(screen.getByTestId('processing-transcript').textContent).not.toContain(
      'the final answer'
    );
    // Tool rows render.
    expect(screen.getAllByTestId('processing-tool-row').length).toBeGreaterThan(0);
    // The sub-agent's own reasoning trail renders beneath.
    const subagents = screen.getByTestId('past-turn-subagents');
    expect(subagents.textContent).toContain('child reasoning trail');
  });
});
