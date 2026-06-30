import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import { store } from '../../../../store';
import type { ToolTimelineEntry } from '../../../../store/chatRuntimeSlice';
import { SubagentActivityBlock, ToolTimelineBlock } from '../ToolTimelineBlock';

// #1122 — guards the parent-thread live subagent rendering. The block
// always expands subagent rows so the activity stays visible while the
// run is in flight, even before the subagent emits any prompt detail.

function renderInStore(ui: React.ReactNode) {
  return render(<Provider store={store}>{ui}</Provider>);
}

describe('SubagentActivityBlock', () => {
  it('renders mode + dedicated-thread + child-turn pills', () => {
    renderInStore(
      <SubagentActivityBlock
        subagent={{
          taskId: 't',
          agentId: 'researcher',
          mode: 'typed',
          dedicatedThread: true,
          childIteration: 2,
          childMaxIterations: 5,
          toolCalls: [],
        }}
      />
    );
    const block = screen.getByTestId('subagent-activity');
    expect(block.textContent).toContain('typed');
    expect(block.textContent).toContain('worker thread');
    expect(block.textContent).toContain('turn 2/5');
  });

  it('renders "step N" when childMaxIterations is null (extended policy)', () => {
    renderInStore(
      <SubagentActivityBlock
        subagent={{ taskId: 't', agentId: 'code_executor', childIteration: 7, toolCalls: [] }}
      />
    );
    const block = screen.getByTestId('subagent-activity');
    expect(block.textContent).toContain('step 7');
    expect(block.textContent).not.toContain('/');
  });

  it('renders final-run statistics on a completed sub-agent', () => {
    renderInStore(
      <SubagentActivityBlock
        subagent={{
          taskId: 't',
          agentId: 'researcher',
          iterations: 3,
          elapsedMs: 4200,
          toolCalls: [],
        }}
      />
    );
    const block = screen.getByTestId('subagent-activity');
    expect(block.textContent).toContain('3 turns');
    expect(block.textContent).toContain('4.2s');
  });

  it('renders one row per child tool call with formatted names, status + timing', () => {
    renderInStore(
      <SubagentActivityBlock
        subagent={{
          taskId: 't',
          agentId: 'researcher',
          toolCalls: [
            { callId: 'c1', toolName: 'web_search', status: 'success', elapsedMs: 312 },
            { callId: 'c2', toolName: 'composio_execute', status: 'running', iteration: 2 },
            { callId: 'c3', toolName: 'file_read', status: 'error', elapsedMs: 50 },
          ],
        }}
      />
    );
    const calls = screen.getAllByTestId('subagent-tool-call');
    expect(calls).toHaveLength(3);
    // Human labels + timing, with status as a tinted "Done" / "Failed" /
    // "Running" tag instead of a bare ✓/✕ glyph or the raw lowercase word.
    expect(calls[0].textContent).toContain('Searching the web');
    expect(calls[0].textContent).toContain('Done');
    expect(calls[0].textContent).toContain('312ms');
    expect(calls[1].textContent).toContain('Composio Execute');
    expect(calls[1].textContent).toContain('Running');
    expect(calls[1].textContent).not.toContain('·t2');
    expect(calls[2].textContent).toContain('Reading file');
    expect(calls[2].textContent).toContain('Failed');
    expect(calls[2].textContent).toContain('50ms');
  });

  it('labels cancelled / awaiting-user calls distinctly (not the green "Done" pill)', () => {
    renderInStore(
      <SubagentActivityBlock
        subagent={{
          taskId: 't',
          agentId: 'researcher',
          toolCalls: [
            { callId: 'c1', toolName: 'web_search', status: 'cancelled', elapsedMs: 10 },
            { callId: 'c2', toolName: 'file_read', status: 'awaiting_user' },
          ],
        }}
      />
    );
    const calls = screen.getAllByTestId('subagent-tool-call');
    expect(calls).toHaveLength(2);
    // A cancelled / awaiting-user call must NOT read as a successful "Done" step.
    expect(calls[0].textContent).toContain('Cancelled');
    expect(calls[0].textContent).not.toContain('Done');
    expect(calls[1].textContent).toContain('Awaiting input');
    expect(calls[1].textContent).not.toContain('Done');
  });

  it('prefers the server-supplied label + contextual detail for a child tool call', () => {
    renderInStore(
      <SubagentActivityBlock
        subagent={{
          taskId: 't',
          agentId: 'researcher',
          toolCalls: [
            {
              callId: 'c1',
              toolName: 'GMAIL_READ_MESSAGES',
              status: 'success',
              displayName: 'Reading messages',
              detail: 'steven@gmail.com',
            },
          ],
        }}
      />
    );
    const row = screen.getByTestId('subagent-tool-call');
    expect(row.textContent).toContain('Reading messages');
    expect(row.textContent).toContain('steven@gmail.com');
    // Never the raw snake_case slug.
    expect(row.textContent).not.toContain('GMAIL_READ_MESSAGES');
  });

  it('renders every thought inline as quoted prose (reasoning + narration)', () => {
    renderInStore(
      <SubagentActivityBlock
        subagent={{
          taskId: 't',
          agentId: 'researcher',
          toolCalls: [],
          transcript: [
            { kind: 'thinking', iteration: 1, text: 'pondering the request' },
            { kind: 'text', iteration: 1, text: 'Here is what I found so far about the topic' },
          ],
        }}
      />
    );
    const thoughts = screen.getAllByTestId('subagent-thought');
    // Both reasoning and visible narration surface as their own prose block —
    // shown directly, with no "Thoughts" heading.
    expect(thoughts).toHaveLength(2);
    expect(thoughts[0].textContent).toContain('pondering the request');
    expect(thoughts[0].textContent).not.toContain('Thoughts');
    expect(thoughts[1].textContent).toContain('Here is what I found so far');
  });

  it('renders thoughts and tool calls interleaved in transcript order', () => {
    renderInStore(
      <SubagentActivityBlock
        subagent={{
          taskId: 't',
          agentId: 'researcher',
          toolCalls: [],
          transcript: [
            { kind: 'thinking', iteration: 1, text: 'I should search the web first' },
            { kind: 'tool', iteration: 1, callId: 'c1', toolName: 'web_search', status: 'success' },
            { kind: 'text', iteration: 1, text: 'Found three relevant results' },
          ],
        }}
      />
    );
    const rows = screen.getByTestId('subagent-transcript').children;
    // Order is preserved: thought → tool → thought.
    expect(rows[0]).toHaveAttribute('data-testid', 'subagent-thought');
    expect(rows[0].textContent).toContain('I should search the web first');
    expect(rows[1]).toHaveAttribute('data-testid', 'subagent-tool-call');
    expect(rows[1].textContent).toContain('Searching the web');
    expect(rows[2]).toHaveAttribute('data-testid', 'subagent-thought');
    expect(rows[2].textContent).toContain('Found three relevant results');
  });

  it('shows a thought directly as prose — no heading, no collapse', () => {
    renderInStore(
      <SubagentActivityBlock
        subagent={{
          taskId: 't',
          agentId: 'researcher',
          toolCalls: [],
          transcript: [{ kind: 'thinking', iteration: 1, text: 'weighing the options' }],
        }}
      />
    );
    const thought = screen.getByTestId('subagent-thought');
    // No collapsible <details>/<summary> and no "Thoughts" heading — the text
    // is shown directly.
    expect(thought.tagName).not.toBe('DETAILS');
    expect(thought.querySelector('summary')).toBeNull();
    expect(thought.textContent).toContain('weighing the options');
    expect(thought.textContent).not.toContain('Thoughts');
    expect(thought.textContent).not.toContain('💭');
  });

  it('strips a leaked <tool_call> envelope from the thought text', () => {
    renderInStore(
      <SubagentActivityBlock
        subagent={{
          taskId: 't',
          agentId: 'researcher',
          toolCalls: [],
          transcript: [
            {
              kind: 'text',
              iteration: 1,
              text: 'I\'ll search your Notion for that. <tool_call> {"name": "NOTION_SEARCH", "arguments": {"query": "audit"}} </tool_call>',
            },
          ],
        }}
      />
    );
    const thought = screen.getByTestId('subagent-thought');
    expect(thought.textContent).toContain("I'll search your Notion for that.");
    // The raw tool-call envelope must not leak into the displayed prose.
    expect(thought.textContent).not.toContain('tool_call');
    expect(thought.textContent).not.toContain('NOTION_SEARCH');
  });

  it('skips an all-whitespace thought delta', () => {
    renderInStore(
      <SubagentActivityBlock
        subagent={{
          taskId: 't',
          agentId: 'researcher',
          toolCalls: [],
          transcript: [{ kind: 'thinking', iteration: 1, text: '   \n  ' }],
        }}
      />
    );
    expect(screen.queryByTestId('subagent-thought')).toBeNull();
  });

  it('renders the view-processing button only when onView is provided', async () => {
    const onView = vi.fn();
    const { rerender } = renderInStore(
      <SubagentActivityBlock subagent={{ taskId: 't', agentId: 'researcher', toolCalls: [] }} />
    );
    expect(screen.queryByTestId('subagent-view-processing')).toBeNull();

    rerender(
      <Provider store={store}>
        <SubagentActivityBlock
          subagent={{ taskId: 't', agentId: 'researcher', toolCalls: [] }}
          onView={onView}
        />
      </Provider>
    );
    const btn = screen.getByTestId('subagent-view-processing');
    await userEvent.click(btn);
    expect(onView).toHaveBeenCalledTimes(1);
  });

  it('renders the inline worktree block + actions when worktreePath is set (#3376)', () => {
    renderInStore(
      <SubagentActivityBlock
        subagent={{
          taskId: 't',
          agentId: 'coder',
          toolCalls: [],
          worktreePath: '/r/.claude/worktrees/worker-a',
          changedFiles: ['src/lib.rs'],
          isDirty: true,
        }}
      />
    );
    const block = screen.getByTestId('subagent-worktree');
    expect(block).toBeInTheDocument();
    // Compact label shows the basename, not the full path.
    expect(block).toHaveTextContent('worker-a');
    expect(screen.getByTestId('worktree-actions')).toBeInTheDocument();
    expect(screen.getByTestId('worktree-remove')).toBeInTheDocument();
  });

  it('omits the worktree block for a non-isolated subagent', () => {
    renderInStore(
      <SubagentActivityBlock subagent={{ taskId: 't', agentId: 'researcher', toolCalls: [] }} />
    );
    expect(screen.queryByTestId('subagent-worktree')).toBeNull();
  });
});

describe('ToolTimelineBlock — agentic task insights surface', () => {
  it('wraps rows in the "Agentic task insights" group and conveys run state on the name', () => {
    const entries: ToolTimelineEntry[] = [
      { id: 'r', name: 'web_search', round: 1, status: 'running', argsBuffer: '{"query":"f1"}' },
      {
        id: 'd',
        name: 'file_read',
        round: 1,
        status: 'success',
        argsBuffer: '{"path":"/a/b.txt"}',
      },
    ];
    renderInStore(<ToolTimelineBlock entries={entries} />);
    const group = screen.getByTestId('agent-task-insights');
    expect(group).toBeInTheDocument();
    // Static section label — NOT a duplicate "Working…" string (the live
    // state lives on the pulsing row names, not the header).
    expect(group.textContent).toContain('Agentic task insights');
    expect(group.textContent).not.toContain('Working');
    // Two rows on the timeline rail.
    expect(screen.getAllByTestId('agent-timeline-row')).toHaveLength(2);
    // Running row name pulses; done row name is solid.
    const running = screen.getByText('Searching: f1');
    const done = screen.getByText('Reading file');
    expect(running.className).toContain('animate-pulse');
    expect(done.className).not.toContain('animate-pulse');
  });

  it('renders nothing for an empty timeline', () => {
    const { container } = renderInStore(<ToolTimelineBlock entries={[]} />);
    expect(container.querySelector('[data-testid="agent-task-insights"]')).toBeNull();
  });

  it('renders the parent live response inside the panel under a Response heading', () => {
    const entries: ToolTimelineEntry[] = [
      { id: 'r', name: 'web_search', round: 1, status: 'running', argsBuffer: '{"query":"f1"}' },
    ];
    renderInStore(
      <ToolTimelineBlock
        entries={entries}
        liveResponse="Let me check your Notion for that audit file."
      />
    );
    const resp = screen.getByTestId('agent-live-response');
    expect(resp.textContent).toContain('Response');
    expect(resp.textContent).toContain('Let me check your Notion for that audit file.');
  });

  it('omits the Response block when there is no live response', () => {
    const entries: ToolTimelineEntry[] = [
      { id: 'r', name: 'web_search', round: 1, status: 'running' },
    ];
    renderInStore(<ToolTimelineBlock entries={entries} />);
    expect(screen.queryByTestId('agent-live-response')).toBeNull();
  });

  it('strips a leaked <tool_call> envelope from the live response', () => {
    const entries: ToolTimelineEntry[] = [
      { id: 'r', name: 'web_search', round: 1, status: 'running' },
    ];
    renderInStore(
      <ToolTimelineBlock
        entries={entries}
        liveResponse={'Searching now. <tool_call> {"name":"X"} </tool_call>'}
      />
    );
    const resp = screen.getByTestId('agent-live-response');
    expect(resp.textContent).toContain('Searching now.');
    expect(resp.textContent).not.toContain('tool_call');
  });
});

describe('ToolTimelineBlock — subagent rendering', () => {
  it('expands a subagent row even without prompt detail and shows child tool calls', () => {
    const entry: ToolTimelineEntry = {
      id: 'tid:subagent:sub-1:researcher',
      name: 'subagent:researcher',
      round: 1,
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
    renderInStore(<ToolTimelineBlock entries={[entry]} />);

    const calls = screen.getAllByTestId('subagent-tool-call');
    expect(calls).toHaveLength(1);
    expect(calls[0].textContent).toContain('Searching the web');
    expect(screen.getByTestId('subagent-activity').textContent).toContain('turn 1/5');
  });

  it('renders a non-subagent row without crashing when there is no detail', () => {
    const entry: ToolTimelineEntry = {
      id: 'plain',
      name: 'list_threads',
      round: 0,
      status: 'success',
    };
    renderInStore(<ToolTimelineBlock entries={[entry]} />);
    // Plain rows with no detail collapse to a flat label + status pill.
    expect(screen.queryByTestId('subagent-activity')).toBeNull();
  });
});

// Issue #1624: when a parent timeline entry contains a worker_thread_ref
// envelope, ToolTimelineBlock must propagate the entry's status to the
// rendered WorkerThreadRefCard so the card's badge stays in lockstep
// with the surrounding `<details>` status pill — both are mutated by
// the same subagent_spawned / subagent_completed / subagent_failed
// socket events.
describe('ToolTimelineBlock — worker thread ref status propagation', () => {
  const WORKER_REF_DETAIL = `summary text\n[worker_thread_ref]\n${JSON.stringify({
    thread_id: 't-worker-1',
    label: 'researcher',
    agent_id: 'researcher',
    task_id: 'task-42',
  })}\n[/worker_thread_ref]`;

  function entryWithStatus(status: ToolTimelineEntry['status']): ToolTimelineEntry {
    return {
      id: `tid:subagent:task-42:researcher:${status}`,
      name: 'subagent:researcher',
      round: 1,
      status,
      detail: WORKER_REF_DETAIL,
    };
  }

  it('passes `running` to the card when the parent entry is in flight', () => {
    renderInStore(<ToolTimelineBlock entries={[entryWithStatus('running')]} />);
    const badge = screen.getByTestId('worker-thread-status-badge');
    expect(badge.getAttribute('data-status')).toBe('running');
  });

  it('passes `completed` to the card when the parent entry succeeds', () => {
    renderInStore(<ToolTimelineBlock entries={[entryWithStatus('success')]} />);
    const badge = screen.getByTestId('worker-thread-status-badge');
    expect(badge.getAttribute('data-status')).toBe('completed');
  });

  it('passes `failed` to the card when the parent entry errors', () => {
    renderInStore(<ToolTimelineBlock entries={[entryWithStatus('error')]} />);
    const badge = screen.getByTestId('worker-thread-status-badge');
    expect(badge.getAttribute('data-status')).toBe('failed');
  });

  // Defensive fallback: if the entry arrives with an unrecognised status
  // (e.g. the union grows in the future, or a malformed payload slips
  // through), the card is rendered as label-only so it can never display a
  // misleading lifecycle state. The status badge must be absent in that case.
  it('omits the status badge when the parent entry has an unknown status', () => {
    const malformed = {
      ...entryWithStatus('success'),
      status: 'queued' as unknown as ToolTimelineEntry['status'],
    };
    renderInStore(<ToolTimelineBlock entries={[malformed]} />);
    expect(screen.queryByTestId('worker-thread-status-badge')).toBeNull();
  });
});

describe('ToolTimelineBlock — compact chat mode (onViewDetails)', () => {
  const entries: ToolTimelineEntry[] = [
    // A finished step.
    { id: 'tl-1', name: 'agent_prepare_context', round: 1, status: 'success', detail: 'fetch X' },
    // The currently-running sub-agent (latest running).
    {
      id: 'sa-1',
      name: 'subagent:researcher',
      round: 1,
      status: 'running',
      subagent: {
        taskId: 'task-1',
        agentId: 'researcher',
        toolCalls: [],
        transcript: [{ kind: 'thinking', iteration: 1, text: 'pondering' }],
      },
    },
  ];

  it('collapses finished steps to a "View details" link but keeps the running step expanded inline', () => {
    const onViewDetails = vi.fn();
    renderInStore(<ToolTimelineBlock entries={entries} onViewDetails={onViewDetails} />);

    // Only the finished step collapses to a "View details →" link.
    const links = screen.getAllByTestId('view-details');
    expect(links).toHaveLength(1);

    // The currently-running sub-agent stays expanded inline in the main UI
    // (its activity is visible) — and shows no "View details" link itself.
    const activity = screen.getByTestId('subagent-activity');
    expect(activity.textContent).toContain('pondering');

    // Clicking the finished step's link opens the full-run panel.
    fireEvent.click(links[0]);
    expect(onViewDetails).toHaveBeenCalledTimes(1);
  });

  it('collapses an already-finished sub-agent (no longer running) to a "View details" link', () => {
    const onViewDetails = vi.fn();
    renderInStore(
      <ToolTimelineBlock
        entries={[
          {
            id: 'sa-done',
            name: 'subagent:researcher',
            round: 1,
            status: 'success',
            subagent: {
              taskId: 'task-2',
              agentId: 'researcher',
              toolCalls: [],
              transcript: [{ kind: 'thinking', iteration: 1, text: 'done thinking' }],
            },
          },
        ]}
        onViewDetails={onViewDetails}
      />
    );
    // No running step → the finished sub-agent collapses (no inline activity).
    expect(screen.getByTestId('view-details')).toBeInTheDocument();
    expect(screen.queryByTestId('subagent-activity')).toBeNull();
  });

  it('still expands inline (no compact link) when onViewDetails is omitted (panel mode)', () => {
    renderInStore(<ToolTimelineBlock entries={entries} expandAllRows />);
    // Panel/expandable path: sub-agent activity is shown, no "View details" link.
    expect(screen.getByTestId('subagent-activity')).toBeInTheDocument();
    expect(screen.queryByTestId('view-details')).toBeNull();
  });
});
