import { fireEvent, render, screen, within } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import { store } from '../../../store';
import type { ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import { ToolTimelineAdapter } from './ToolTimelineAdapter';

// `WorkerThreadRefCard` (rendered for a `[worker_thread_ref]` envelope) reads
// through `useAppDispatch`, so those cases render inside a real store.
function renderInStore(ui: React.ReactNode) {
  return render(<Provider store={store}>{ui}</Provider>);
}

/**
 * Ports the meaningful behavior coverage from the deleted
 * `components/__tests__/ToolTimelineBlock.test.tsx` onto the vendored
 * `elements/tool-timeline`-hosted adapter. Windowing/auto-scroll is tested at
 * the "reports the windowed attribute and doesn't throw on scroll" level, per
 * the migration brief — the underlying `ResizeObserver` wiring is unchanged
 * from the deleted component.
 */
describe('ToolTimelineAdapter — agentic task insights surface', () => {
  it('wraps rows in the "Agentic task insights" group and conveys run state on the name', () => {
    const entries: ToolTimelineEntry[] = [
      {
        id: 'r',
        name: 'web_search',
        round: 1,
        seq: 0,
        status: 'running',
        argsBuffer: '{"query":"f1"}',
      },
      {
        id: 'd',
        name: 'file_read',
        round: 1,
        seq: 0,
        status: 'success',
        argsBuffer: '{"path":"/a/b.txt"}',
      },
    ];
    render(<ToolTimelineAdapter entries={entries} />);
    const group = screen.getByTestId('agent-task-insights');
    expect(group).toBeInTheDocument();
    expect(group.textContent).toContain('Agentic task insights');
    expect(group.textContent).not.toContain('Working');
    expect(screen.getAllByTestId('agent-timeline-row')).toHaveLength(2);
    const running = screen.getByText('Searching the web');
    const done = screen.getByText('Read file');
    expect(running.className).toContain('animate-pulse');
    expect(done.className).not.toContain('animate-pulse');
  });

  it('renders rows in seq (issue) order, not array (arrival) order', () => {
    const entries: ToolTimelineEntry[] = [
      { id: 'third', name: 'run_code', round: 1, seq: 2, status: 'success' },
      { id: 'first', name: 'web_search', round: 1, seq: 0, status: 'success' },
      { id: 'second', name: 'file_read', round: 1, seq: 1, status: 'success' },
    ];
    render(<ToolTimelineAdapter entries={entries} />);
    const rows = screen.getAllByTestId('agent-timeline-row');
    expect(rows).toHaveLength(3);
    expect(rows[0].textContent).toContain('Searched the web');
    expect(rows[1].textContent).toContain('Read file');
    expect(rows[2].textContent).toContain('Ran code');
  });

  it('renders nothing for an empty timeline', () => {
    const { container } = render(<ToolTimelineAdapter entries={[]} />);
    expect(container.querySelector('[data-testid="agent-task-insights"]')).toBeNull();
  });

  it('stays open while running and collapses once settled so a finished run does not dominate', () => {
    const running: ToolTimelineEntry[] = [
      { id: 'r', name: 'web_search', round: 1, seq: 0, status: 'running' },
    ];
    const { rerender } = render(<ToolTimelineAdapter entries={running} />);
    expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('data-state', 'open');

    const settled: ToolTimelineEntry[] = [
      { id: 'r', name: 'web_search', round: 1, seq: 0, status: 'success' },
    ];
    rerender(<ToolTimelineAdapter entries={settled} />);
    expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('data-state', 'closed');

    // The side panel still forces every row open via expandAllRows.
    rerender(<ToolTimelineAdapter entries={settled} expandAllRows />);
    expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('data-state', 'open');
  });

  it('resets a user expand when a new turn settles, but sticks while the turn is still running (#4942/#5008)', () => {
    const turn1Settled: ToolTimelineEntry[] = [
      { id: 't1', name: 'web_search', round: 1, seq: 0, status: 'success' },
    ];
    const { rerender } = render(<ToolTimelineAdapter entries={turn1Settled} />);
    expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('data-state', 'closed');

    fireEvent.click(screen.getByRole('button', { name: 'Agentic task insights' }));
    expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('data-state', 'open');

    const turn2Running: ToolTimelineEntry[] = [
      ...turn1Settled,
      { id: 't2', name: 'file_read', round: 2, seq: 1, status: 'running' },
    ];
    rerender(<ToolTimelineAdapter entries={turn2Running} />);
    expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('data-state', 'open');

    const turn2Settled: ToolTimelineEntry[] = [
      ...turn1Settled,
      { id: 't2', name: 'file_read', round: 2, seq: 1, status: 'success' },
    ];
    rerender(<ToolTimelineAdapter entries={turn2Settled} />);
    expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('data-state', 'closed');
  });

  it('with turnActive: does not reset the user override on an isRunning toggle within the same turn, only when turnActive itself goes false', () => {
    const subagentARunning: ToolTimelineEntry[] = [
      { id: 'a', name: 'subagent:researcher', round: 1, seq: 0, status: 'running' },
    ];
    const { rerender } = render(<ToolTimelineAdapter entries={subagentARunning} turnActive />);
    expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('data-state', 'open');

    fireEvent.click(screen.getByRole('button', { name: 'Agentic task insights' }));
    expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('data-state', 'closed');

    const subagentBRunning: ToolTimelineEntry[] = [
      { id: 'a', name: 'subagent:researcher', round: 1, seq: 0, status: 'success' },
      { id: 'b', name: 'subagent:coder', round: 1, seq: 1, status: 'running' },
    ];
    rerender(<ToolTimelineAdapter entries={subagentBRunning} turnActive />);
    // Still one turn — the user's collapse must hold despite isRunning toggling.
    expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('data-state', 'closed');

    rerender(<ToolTimelineAdapter entries={subagentBRunning} turnActive={false} />);
    expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('data-state', 'closed');
  });

  it('renders the tool result output inside the expanded row', () => {
    const entries: ToolTimelineEntry[] = [
      {
        id: 'd',
        name: 'web_search',
        round: 1,
        seq: 0,
        status: 'success',
        argsBuffer: '{"query":"f1"}',
        result: 'Top result: https://openhuman.dev',
      },
    ];
    render(<ToolTimelineAdapter entries={entries} expandAllRows />);
    expect(screen.getByTestId('tool-result-output').textContent).toContain(
      'Top result: https://openhuman.dev'
    );
  });

  it('renders the parent live response inside the panel under a Response heading, stripping a leaked tool_call envelope', () => {
    const entries: ToolTimelineEntry[] = [
      {
        id: 'r',
        name: 'web_search',
        round: 1,
        seq: 0,
        status: 'running',
        argsBuffer: '{"query":"f1"}',
      },
    ];
    render(
      <ToolTimelineAdapter
        entries={entries}
        liveResponse={'Searching now. <tool_call> {"name":"X"} </tool_call>'}
      />
    );
    const resp = screen.getByTestId('agent-live-response');
    expect(resp.textContent).toContain('Response');
    expect(resp.textContent).toContain('Searching now.');
    expect(resp.textContent).not.toContain('tool_call');
  });

  it('omits the Response block when there is no live response', () => {
    render(
      <ToolTimelineAdapter
        entries={[{ id: 'r', name: 'web_search', round: 1, seq: 0, status: 'running' }]}
      />
    );
    expect(screen.queryByTestId('agent-live-response')).toBeNull();
  });
});

describe('ToolTimelineAdapter — coalescing repeated rows', () => {
  it('collapses consecutive identical body-less rows into one ×N row', () => {
    const entries: ToolTimelineEntry[] = Array.from({ length: 5 }, (_, i) => ({
      id: `dup-${i}`,
      name: 'integrations_agent',
      round: 1,
      seq: 0,
      status: 'success' as const,
    }));
    render(<ToolTimelineAdapter entries={entries} />);
    expect(screen.getAllByTestId('agent-timeline-row')).toHaveLength(1);
    expect(screen.getByTestId('timeline-repeat-count').textContent).toBe('×5');
  });

  it('does not merge across differing status or the live running row', () => {
    const entries: ToolTimelineEntry[] = [
      { id: 'a', name: 'integrations_agent', round: 1, seq: 0, status: 'success' },
      { id: 'b', name: 'integrations_agent', round: 1, seq: 0, status: 'success' },
      { id: 'c', name: 'integrations_agent', round: 1, seq: 0, status: 'error' },
      { id: 'd', name: 'integrations_agent', round: 1, seq: 0, status: 'running' },
    ];
    render(<ToolTimelineAdapter entries={entries} />);
    expect(screen.getAllByTestId('agent-timeline-row')).toHaveLength(3);
    const counts = screen.getAllByTestId('timeline-repeat-count');
    expect(counts).toHaveLength(1);
    expect(counts[0].textContent).toBe('×2');
  });
});

describe('ToolTimelineAdapter — subagent rendering', () => {
  it('shows child tool calls after the collapsed subagent row is opened', () => {
    const entry: ToolTimelineEntry = {
      id: 'tid:subagent:sub-1:researcher',
      name: 'subagent:researcher',
      round: 1,
      seq: 0,
      status: 'running',
      subagent: {
        taskId: 'sub-1',
        agentId: 'researcher',
        mode: 'typed',
        childIteration: 1,
        childMaxIterations: 5,
        toolCalls: [{ callId: 'cc-1', toolName: 'web_search', status: 'running', iteration: 1 }],
      },
    };
    render(<ToolTimelineAdapter entries={[entry]} />);

    const subagent = screen.getByTestId('assistant-ui-subagent-call');
    const trigger = within(subagent).getByRole('button');
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(trigger);
    expect(screen.getByTestId('subagent-activity')).toBeInTheDocument();
  });

  it('renders a non-subagent row without crashing when there is no detail', () => {
    render(
      <ToolTimelineAdapter
        entries={[{ id: 'plain', name: 'list_threads', round: 0, seq: 0, status: 'success' }]}
      />
    );
    expect(screen.queryByTestId('subagent-activity')).toBeNull();
  });
});

// Issue #1624: a worker_thread_ref envelope propagates the parent entry's
// status onto the rendered WorkerThreadRefCard's badge.
describe('ToolTimelineAdapter — worker thread ref status propagation', () => {
  const WORKER_REF_DETAIL = `summary text\n[worker_thread_ref]\n${JSON.stringify({
    thread_id: 't-worker-1',
    label: 'code_executor',
    agent_id: 'code_executor',
    task_id: 'task-42',
  })}\n[/worker_thread_ref]`;

  function entryWithStatus(status: ToolTimelineEntry['status']): ToolTimelineEntry {
    return {
      id: `tid:subagent:task-42:code_executor:${status}`,
      name: 'subagent:code_executor',
      round: 1,
      seq: 0,
      status,
      detail: WORKER_REF_DETAIL,
    };
  }

  it('passes `running` to the card when the parent entry is in flight', () => {
    renderInStore(<ToolTimelineAdapter entries={[entryWithStatus('running')]} />);
    expect(screen.getByTestId('worker-thread-status-badge').getAttribute('data-status')).toBe(
      'running'
    );
  });

  it('passes `completed` to the card when the parent entry succeeds', () => {
    renderInStore(<ToolTimelineAdapter entries={[entryWithStatus('success')]} />);
    expect(screen.getByTestId('worker-thread-status-badge').getAttribute('data-status')).toBe(
      'completed'
    );
  });

  it('passes `failed` to the card when the parent entry errors', () => {
    renderInStore(<ToolTimelineAdapter entries={[entryWithStatus('error')]} />);
    expect(screen.getByTestId('worker-thread-status-badge').getAttribute('data-status')).toBe(
      'failed'
    );
  });
});

describe('ToolTimelineAdapter — compact chat mode (onViewDetails)', () => {
  const entries: ToolTimelineEntry[] = [
    {
      id: 'tl-1',
      name: 'read_file',
      round: 1,
      seq: 0,
      status: 'success',
      detail: 'notes.md',
      result: 'Read 3 lines.',
    },
    {
      id: 'sa-1',
      name: 'subagent:researcher',
      round: 1,
      seq: 0,
      status: 'running',
      subagent: {
        taskId: 'task-1',
        agentId: 'researcher',
        toolCalls: [],
        transcript: [{ kind: 'thinking', iteration: 1, text: 'pondering' }],
      },
    },
  ];

  it('collapses finished steps to a link and keeps the running delegation card inline', () => {
    const onViewDetails = vi.fn();
    render(<ToolTimelineAdapter entries={entries} onViewDetails={onViewDetails} />);

    const links = screen.getAllByTestId('view-details');
    expect(links).toHaveLength(1);

    const subagent = screen.getByTestId('assistant-ui-subagent-call');
    fireEvent.click(within(subagent).getByRole('button'));
    expect(screen.getByTestId('subagent-activity')).toBeInTheDocument();

    fireEvent.click(links[0]);
    expect(onViewDetails).toHaveBeenCalledTimes(1);
  });

  it('still expands inline (no compact link) when onViewDetails is omitted (panel mode)', () => {
    render(<ToolTimelineAdapter entries={entries} expandAllRows />);
    expect(screen.queryByTestId('view-details')).toBeNull();
  });
});

describe('ToolTimelineAdapter — in-flight viewport windowing', () => {
  const runningEntries: ToolTimelineEntry[] = [
    { id: 'w-1', name: 'read_file', round: 1, seq: 0, status: 'success', detail: 'a.ts' },
    { id: 'w-2', name: 'code_executor', round: 1, seq: 1, status: 'running', detail: 'run' },
  ];

  it('windows the row list while the turn is active', () => {
    render(<ToolTimelineAdapter entries={runningEntries} turnActive />);
    const viewport = screen.getByTestId('tool-timeline-viewport');
    expect(viewport.getAttribute('data-windowed')).toBe('true');
    expect(viewport.className).toContain('overflow-y-auto');
  });

  it('does not window once the turn has settled', () => {
    render(<ToolTimelineAdapter entries={runningEntries} turnActive={false} />);
    expect(screen.getByTestId('tool-timeline-viewport').getAttribute('data-windowed')).toBe(
      'false'
    );
  });

  it('never windows under expandAllRows, even mid-turn', () => {
    render(<ToolTimelineAdapter entries={runningEntries} turnActive expandAllRows />);
    expect(screen.getByTestId('tool-timeline-viewport').getAttribute('data-windowed')).toBe(
      'false'
    );
  });

  it('attaches a scroll handler that does not throw as scroll metrics change', () => {
    render(<ToolTimelineAdapter entries={runningEntries} turnActive />);
    const viewport = screen.getByTestId('tool-timeline-viewport');
    Object.defineProperty(viewport, 'scrollHeight', { value: 500, configurable: true });
    Object.defineProperty(viewport, 'clientHeight', { value: 100, configurable: true });
    viewport.scrollTop = 0;
    expect(() => fireEvent.scroll(viewport)).not.toThrow();
    viewport.scrollTop = 400;
    expect(() => fireEvent.scroll(viewport)).not.toThrow();
  });
});

describe('ToolTimelineAdapter — renders the processing transcript inline', () => {
  it('renders narration and tool steps from the transcript', () => {
    render(
      <ToolTimelineAdapter
        entries={[{ id: 'c1', name: 'file_read', round: 1, seq: 0, status: 'success' }]}
        transcript={[
          { kind: 'narration', round: 1, seq: 0, text: 'Let me check that file.' },
          { kind: 'toolCall', round: 1, seq: 1, callId: 'c1' },
        ]}
      />
    );
    expect(screen.getByTestId('processing-transcript')).toBeInTheDocument();
    expect(screen.getByText('Let me check that file.')).toBeInTheDocument();
  });

  it('shows an available tool-result diagnostic for unknown failures in the processing transcript', () => {
    const entries: ToolTimelineEntry[] = [
      {
        id: 'failed-tool',
        name: 'run_code',
        round: 1,
        seq: 1,
        status: 'error',
        result: 'Process exited with status 127: command not found.',
        failure: {
          class: 'Unknown',
          category: 'Permanent',
          recoverable: false,
          causePlain: 'Generic failure.',
          nextAction: 'Try again.',
        },
      },
    ];
    render(
      <ToolTimelineAdapter
        entries={entries}
        transcript={[{ kind: 'toolCall', round: 1, seq: 1, callId: 'failed-tool' }]}
      />
    );

    const card = screen.getByTestId('assistant-ui-tool-failure');
    expect(card).toHaveTextContent('Process exited with status 127: command not found.');
    expect(card).not.toHaveTextContent('Generic failure.');
  });

  it('falls back to the tool-row list when no transcript is present', () => {
    render(
      <ToolTimelineAdapter
        entries={[{ id: 'a', name: 'web_search', round: 1, seq: 0, status: 'success' }]}
      />
    );
    expect(screen.queryByTestId('processing-transcript')).toBeNull();
    expect(screen.getByTestId('agent-timeline-row')).toBeInTheDocument();
  });

  it('renders on transcript alone, with no tool rows yet', () => {
    render(
      <ToolTimelineAdapter
        entries={[]}
        transcript={[{ kind: 'narration', round: 1, seq: 0, text: 'Thinking about the request.' }]}
      />
    );
    expect(screen.getByTestId('agent-task-insights')).toBeInTheDocument();
  });

  it('still renders nothing when there is neither a row nor transcript prose', () => {
    const { container } = render(<ToolTimelineAdapter entries={[]} transcript={[]} />);
    expect(container.querySelector('[data-testid="agent-task-insights"]')).toBeNull();
  });
});
