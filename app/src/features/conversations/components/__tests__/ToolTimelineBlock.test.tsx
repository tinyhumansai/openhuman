import { fireEvent, render, screen, within } from '@testing-library/react';
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

  it('renders rows in seq (issue) order, not array (arrival) order', () => {
    // Simulates the out-of-order-arrival bug: a `tool_args_delta` for
    // a later parallel call can land — and create its row — before an
    // earlier call's own event, so the entries array ends up scrambled
    // relative to the order the agent actually issued the calls. `seq` is
    // the source of truth for display order; the array position is not.
    const entries: ToolTimelineEntry[] = [
      { id: 'third', name: 'run_code', round: 1, seq: 2, status: 'success' },
      { id: 'first', name: 'web_search', round: 1, seq: 0, status: 'success' },
      { id: 'second', name: 'file_read', round: 1, seq: 1, status: 'success' },
    ];
    renderInStore(<ToolTimelineBlock entries={entries} />);
    const rows = screen.getAllByTestId('agent-timeline-row');
    expect(rows).toHaveLength(3);
    expect(rows[0].textContent).toContain('Searching the web');
    expect(rows[1].textContent).toContain('Reading file');
    expect(rows[2].textContent).toContain('Run Code');
  });

  it('renders nothing for an empty timeline', () => {
    const { container } = renderInStore(<ToolTimelineBlock entries={[]} />);
    expect(container.querySelector('[data-testid="agent-task-insights"]')).toBeNull();
  });

  it('stays open while running and collapses once settled so a finished run does not dominate', () => {
    const running: ToolTimelineEntry[] = [
      { id: 'r', name: 'web_search', round: 1, seq: 0, status: 'running' },
    ];
    const { rerender } = renderInStore(<ToolTimelineBlock entries={running} />);
    // In flight → the group is open so the live activity is visible.
    expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('open');

    // Settled (no running row) → collapsed by default; the rows stay in the DOM
    // one click away, but no longer flood the conversation.
    const settled: ToolTimelineEntry[] = [
      { id: 'r', name: 'web_search', round: 1, seq: 0, status: 'success' },
    ];
    rerender(
      <Provider store={store}>
        <ToolTimelineBlock entries={settled} />
      </Provider>
    );
    expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');

    // The side panel still forces every row open via expandAllRows.
    rerender(
      <Provider store={store}>
        <ToolTimelineBlock entries={settled} expandAllRows />
      </Provider>
    );
    expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('open');
  });

  // Regression coverage for "Agentic task insights keeps collapsing on every
  // new feedback": the workflow copilot keeps ONE `ToolTimelineBlock` mounted
  // for the life of a thread, appending each new turn's entries onto the same
  // `entries` prop (see `WorkflowCopilotPanel`/`useWorkflowBuilderChat`) — it
  // never remounts the block per turn. Before the fix, the outer group's
  // `open` was driven purely by `isRunning || expandAllRows`, so every time a
  // turn settled (running → not running) the group snapped shut regardless of
  // anything the user had done, discarding a manual expand made moments
  // earlier. These tests simulate that same "new turn's entries land on an
  // already-mounted block" shape via `rerender` rather than remounting.
  describe('agentic task insights — sticky user expand/collapse across turns', () => {
    it('resets a user expand when a new turn settles', () => {
      const turn1Settled: ToolTimelineEntry[] = [
        { id: 't1', name: 'web_search', round: 1, seq: 0, status: 'success' },
      ];
      const { rerender } = renderInStore(<ToolTimelineBlock entries={turn1Settled} />);
      // Default: settled and collapsed (unchanged behaviour).
      expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');

      // The user manually expands it.
      fireEvent.click(screen.getByText('Agentic task insights'));
      expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('open');

      // A new turn/feedback starts streaming onto the SAME mounted block. The
      // override still wins WHILE it runs — the user's choice isn't clobbered
      // mid-turn (#4942).
      const turn2Running: ToolTimelineEntry[] = [
        ...turn1Settled,
        { id: 't2', name: 'file_read', round: 2, seq: 1, status: 'running' },
      ];
      rerender(
        <Provider store={store}>
          <ToolTimelineBlock entries={turn2Running} />
        </Provider>
      );
      expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('open');

      // ...and settles. The override only sticks WITHIN a turn — once this
      // turn finishes, the auto-collapse applies to it, so the panel
      // collapses instead of permanently overriding every future turn.
      const turn2Settled: ToolTimelineEntry[] = [
        ...turn1Settled,
        { id: 't2', name: 'file_read', round: 2, seq: 1, status: 'success' },
      ];
      rerender(
        <Provider store={store}>
          <ToolTimelineBlock entries={turn2Settled} />
        </Provider>
      );
      expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');
    });

    it('leaves the default open-while-running/collapsed-when-settled behaviour unchanged absent any user interaction', () => {
      const running: ToolTimelineEntry[] = [
        { id: 'r', name: 'web_search', round: 1, seq: 0, status: 'running' },
      ];
      const { rerender } = renderInStore(<ToolTimelineBlock entries={running} />);
      expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('open');

      const settled: ToolTimelineEntry[] = [
        { id: 'r', name: 'web_search', round: 1, seq: 0, status: 'success' },
      ];
      rerender(
        <Provider store={store}>
          <ToolTimelineBlock entries={settled} />
        </Provider>
      );
      expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');
    });

    it('also persists an explicit user collapse across a new turn (does not force it back open)', () => {
      const turn1Running: ToolTimelineEntry[] = [
        { id: 't1', name: 'web_search', round: 1, seq: 0, status: 'running' },
      ];
      const { rerender } = renderInStore(<ToolTimelineBlock entries={turn1Running} />);
      expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('open');

      // The user collapses it while a turn is still running.
      fireEvent.click(screen.getByText('Agentic task insights'));
      expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');

      // A new turn starts running — the auto rule alone would force it back
      // open, but the user's explicit collapse must win.
      const turn2Running: ToolTimelineEntry[] = [
        { id: 't1', name: 'web_search', round: 1, seq: 0, status: 'success' },
        { id: 't2', name: 'file_read', round: 2, seq: 1, status: 'running' },
      ];
      rerender(
        <Provider store={store}>
          <ToolTimelineBlock entries={turn2Running} />
        </Provider>
      );
      expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');
    });

    it('auto-collapses when a turn finishes even if the user had expanded it', () => {
      const turn1Settled: ToolTimelineEntry[] = [
        { id: 't1', name: 'web_search', round: 1, seq: 0, status: 'success' },
      ];
      const { rerender } = renderInStore(<ToolTimelineBlock entries={turn1Settled} />);
      expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');

      // User expands the settled turn1 panel.
      fireEvent.click(screen.getByText('Agentic task insights'));
      expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('open');

      // A new turn starts running — stays open (both the override and the
      // auto rule agree here).
      const turn2Running: ToolTimelineEntry[] = [
        ...turn1Settled,
        { id: 't2', name: 'file_read', round: 2, seq: 1, status: 'running' },
      ];
      rerender(
        <Provider store={store}>
          <ToolTimelineBlock entries={turn2Running} />
        </Provider>
      );
      expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('open');

      // It settles — the override is cleared on this running→settled edge,
      // so the panel auto-collapses instead of staying pinned open forever.
      const turn2Settled: ToolTimelineEntry[] = [
        ...turn1Settled,
        { id: 't2', name: 'file_read', round: 2, seq: 1, status: 'success' },
      ];
      rerender(
        <Provider store={store}>
          <ToolTimelineBlock entries={turn2Settled} />
        </Provider>
      );
      expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');
    });

    it('does not collapse mid-stream when new entries arrive on a running turn', () => {
      const turn1Running: ToolTimelineEntry[] = [
        { id: 't1', name: 'web_search', round: 1, seq: 0, status: 'running' },
      ];
      const { rerender } = renderInStore(<ToolTimelineBlock entries={turn1Running} />);
      expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('open');

      // More entries stream in while the turn is still running — the
      // running→settled edge never fires, so the panel must stay open.
      const turn1StillRunning: ToolTimelineEntry[] = [
        ...turn1Running,
        { id: 't1b', name: 'file_read', round: 1, seq: 1, status: 'running' },
      ];
      rerender(
        <Provider store={store}>
          <ToolTimelineBlock entries={turn1StillRunning} />
        </Provider>
      );
      expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('open');

      const turn1MoreRunning: ToolTimelineEntry[] = [
        { id: 't1', name: 'web_search', round: 1, seq: 0, status: 'success' },
        { id: 't1b', name: 'file_read', round: 1, seq: 1, status: 'running' },
      ];
      rerender(
        <Provider store={store}>
          <ToolTimelineBlock entries={turn1MoreRunning} />
        </Provider>
      );
      expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('open');
    });

    it('respects a manual expand during an active turn, resets on settle', () => {
      const turn1Running: ToolTimelineEntry[] = [
        { id: 't1', name: 'web_search', round: 1, seq: 0, status: 'running' },
      ];
      const { rerender } = renderInStore(<ToolTimelineBlock entries={turn1Running} />);
      // Auto-open while running (no override yet).
      expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('open');

      // User explicitly collapses it mid-run...
      fireEvent.click(screen.getByText('Agentic task insights'));
      expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');

      // ...then explicitly re-expands it — a manual expand during the still-
      // active turn — and it must stick while the turn keeps running.
      fireEvent.click(screen.getByText('Agentic task insights'));
      expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('open');

      // The turn settles — the manual override resets on this edge, and
      // since the run is done the auto rule collapses the panel.
      const turn1Settled: ToolTimelineEntry[] = [
        { id: 't1', name: 'web_search', round: 1, seq: 0, status: 'success' },
      ];
      rerender(
        <Provider store={store}>
          <ToolTimelineBlock entries={turn1Settled} />
        </Provider>
      );
      expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');
    });
  });

  // #5008 shipped the reset-on-settle fix using the `isRunning` true→false
  // edge, but that edge fires once PER SUB-AGENT within a single turn (each
  // subagent_spawned/subagent_completed pair toggles `isRunning`), not once
  // per turn — so on a multi-sub-agent turn the panel's override reset (and
  // its auto-collapse) fired repeatedly, flickering the panel open/closed
  // as each sub-agent came and went. `turnActive` — sourced from
  // `inferenceTurnLifecycleByThread`, the same lifecycle the chat threads
  // page uses for `isSending` — transitions exactly once per USER TURN, so
  // passing it in makes the reset track the turn instead of any single
  // sub-agent.
  describe('with turnActive prop', () => {
    it('does not reset the user override while turnActive stays true across multiple isRunning toggles, only resetting (and auto-collapsing) when turnActive itself goes false', () => {
      const subagentARunning: ToolTimelineEntry[] = [
        { id: 'a', name: 'subagent:researcher', round: 1, seq: 0, status: 'running' },
      ];
      const { rerender } = renderInStore(
        <ToolTimelineBlock entries={subagentARunning} turnActive />
      );
      // Sub-agent A running, turn active → auto-open (no override yet).
      expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('open');

      // Sub-agent A settles: `isRunning` flips true→false, but the whole
      // TURN is still active, so the group STAYS OPEN.
      //
      // This expectation was inverted deliberately. #5008 moved the override
      // reset onto `turnActive` but left `autoOpen` on `isRunning`, so the
      // group still auto-collapsed in every gap between tools/sub-agents —
      // a just-delivered tool result appeared to be wiped a beat later, and a
      // multi-tool turn flickered. `autoOpen` now tracks the same whole-turn
      // signal as the reset, so the group collapses exactly once, at settle.
      const subagentASettled: ToolTimelineEntry[] = [
        { id: 'a', name: 'subagent:researcher', round: 1, seq: 0, status: 'success' },
      ];
      rerender(
        <Provider store={store}>
          <ToolTimelineBlock entries={subagentASettled} turnActive />
        </Provider>
      );
      expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('open');

      // The user manually COLLAPSES it while the turn is still in flight.
      // (Pre-change the auto rule had already closed it here, so this click
      // was an expand; the override mechanic under test is identical either
      // way — what matters is that the explicit choice survives the toggles
      // below.)
      fireEvent.click(screen.getByText('Agentic task insights'));
      expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');

      // Sub-agent B spawns: `isRunning` flips false→true again. Still one
      // turn (`turnActive` unchanged) — the user's override must hold, so the
      // group stays COLLAPSED despite the auto rule wanting it open.
      const subagentBRunning: ToolTimelineEntry[] = [
        ...subagentASettled,
        { id: 'b', name: 'subagent:coder', round: 1, seq: 1, status: 'running' },
      ];
      rerender(
        <Provider store={store}>
          <ToolTimelineBlock entries={subagentBRunning} turnActive />
        </Provider>
      );
      expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');

      // Sub-agent B settles: `isRunning` flips true→false a second time
      // within the SAME turn. This is exactly the edge that used to reset
      // the override and cause the flicker (#5008 regression) — with
      // `turnActive` supplied it must NOT reset; the user's collapse sticks.
      const subagentBSettled: ToolTimelineEntry[] = [
        ...subagentASettled,
        { id: 'b', name: 'subagent:coder', round: 1, seq: 1, status: 'success' },
      ];
      rerender(
        <Provider store={store}>
          <ToolTimelineBlock entries={subagentBSettled} turnActive />
        </Provider>
      );
      expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');

      // Only when the TURN itself ends (`turnActive` true→false) does the
      // override reset — the panel then auto-collapses since the run is done.
      rerender(
        <Provider store={store}>
          <ToolTimelineBlock entries={subagentBSettled} turnActive={false} />
        </Provider>
      );
      expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');
    });

    it('keeps a mid-turn manual collapse intact across a sub-agent settling, only reopening per the auto rule once turnActive ends', () => {
      const subagentARunning: ToolTimelineEntry[] = [
        { id: 'a', name: 'subagent:researcher', round: 1, seq: 0, status: 'running' },
      ];
      const { rerender } = renderInStore(
        <ToolTimelineBlock entries={subagentARunning} turnActive />
      );
      expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('open');

      // The user explicitly collapses it while sub-agent A is still running.
      fireEvent.click(screen.getByText('Agentic task insights'));
      expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');

      // Sub-agent A settles, sub-agent B spawns and settles too — all within
      // the same turn (`turnActive` stays true throughout). None of these
      // `isRunning` toggles may reopen the panel against the user's choice.
      const afterSubagentB: ToolTimelineEntry[] = [
        { id: 'a', name: 'subagent:researcher', round: 1, seq: 0, status: 'success' },
        { id: 'b', name: 'subagent:coder', round: 1, seq: 1, status: 'running' },
      ];
      rerender(
        <Provider store={store}>
          <ToolTimelineBlock entries={afterSubagentB} turnActive />
        </Provider>
      );
      expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');

      const bothSettled: ToolTimelineEntry[] = [
        { id: 'a', name: 'subagent:researcher', round: 1, seq: 0, status: 'success' },
        { id: 'b', name: 'subagent:coder', round: 1, seq: 1, status: 'success' },
      ];
      rerender(
        <Provider store={store}>
          <ToolTimelineBlock entries={bothSettled} turnActive />
        </Provider>
      );
      expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');

      // The turn ends — override resets; auto rule (settled, not running)
      // keeps it collapsed, same outcome but for the right reason now.
      rerender(
        <Provider store={store}>
          <ToolTimelineBlock entries={bothSettled} turnActive={false} />
        </Provider>
      );
      expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');

      // Both sides of that transition read "collapsed", which a STALE `false`
      // override would also produce — prove the override actually reset (not
      // just that it happened to still agree with the auto rule) by starting
      // a brand-new turn: the auto rule alone (isRunning) should now govern,
      // reopening the panel with no further user interaction.
      const newTurnRunning: ToolTimelineEntry[] = [
        { id: 'c', name: 'subagent:researcher', round: 2, seq: 0, status: 'running' },
      ];
      rerender(
        <Provider store={store}>
          <ToolTimelineBlock entries={newTurnRunning} turnActive />
        </Provider>
      );
      expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('open');
    });
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
    renderInStore(<ToolTimelineBlock entries={entries} expandAllRows />);
    const output = screen.getByTestId('tool-result-output');
    expect(output.textContent).toContain('Top result: https://openhuman.dev');
  });

  it('makes a row expandable on a result alone and omits the block without one', () => {
    const entries: ToolTimelineEntry[] = [
      // No argsBuffer / detail / subagent — the result is the only body.
      { id: 'a', name: 'run_code', round: 1, seq: 0, status: 'success', result: 'exit 0' },
      { id: 'b', name: 'run_code', round: 2, seq: 0, status: 'success' },
    ];
    renderInStore(<ToolTimelineBlock entries={entries} expandAllRows />);
    const outputs = screen.getAllByTestId('tool-result-output');
    expect(outputs).toHaveLength(1);
    expect(outputs[0].textContent).toBe('exit 0');
  });

  it('renders the parent live response inside the panel under a Response heading', () => {
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
      { id: 'r', name: 'web_search', round: 1, seq: 0, status: 'running' },
    ];
    renderInStore(<ToolTimelineBlock entries={entries} />);
    expect(screen.queryByTestId('agent-live-response')).toBeNull();
  });

  it('strips a leaked <tool_call> envelope from the live response', () => {
    const entries: ToolTimelineEntry[] = [
      { id: 'r', name: 'web_search', round: 1, seq: 0, status: 'running' },
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

describe('ToolTimelineBlock — coalescing repeated rows', () => {
  it('collapses consecutive identical body-less rows into one ×N row', () => {
    // A retry loop that spawns the same integrations step five times, each
    // surfacing the generic "Checking your connected app" label with no
    // distinguishing detail/result/subagent.
    const entries: ToolTimelineEntry[] = Array.from({ length: 5 }, (_, i) => ({
      id: `dup-${i}`,
      name: 'integrations_agent',
      round: 1,
      seq: 0,
      status: 'success' as const,
    }));
    renderInStore(<ToolTimelineBlock entries={entries} />);
    // Five entries render as a single rail row carrying an ×5 badge.
    expect(screen.getAllByTestId('agent-timeline-row')).toHaveLength(1);
    expect(screen.getByTestId('timeline-repeat-count').textContent).toBe('×5');
  });

  it('does not merge across differing status or the live running row', () => {
    const entries: ToolTimelineEntry[] = [
      { id: 'a', name: 'integrations_agent', round: 1, seq: 0, status: 'success' },
      { id: 'b', name: 'integrations_agent', round: 1, seq: 0, status: 'success' },
      // Different status breaks the run.
      { id: 'c', name: 'integrations_agent', round: 1, seq: 0, status: 'error' },
      // The live running row is never folded away.
      { id: 'd', name: 'integrations_agent', round: 1, seq: 0, status: 'running' },
    ];
    renderInStore(<ToolTimelineBlock entries={entries} />);
    // success×2 (merged) + error (single) + running (single) = 3 rows.
    expect(screen.getAllByTestId('agent-timeline-row')).toHaveLength(3);
    const counts = screen.getAllByTestId('timeline-repeat-count');
    expect(counts).toHaveLength(1);
    expect(counts[0].textContent).toBe('×2');
  });

  it('never merges rows that carry a unique result body', () => {
    const entries: ToolTimelineEntry[] = [
      { id: 'a', name: 'run_code', round: 1, seq: 0, status: 'success', result: 'exit 0' },
      { id: 'b', name: 'run_code', round: 1, seq: 0, status: 'success', result: 'exit 1' },
    ];
    renderInStore(<ToolTimelineBlock entries={entries} expandAllRows />);
    // Both keep their own row — distinct results are never coalesced.
    expect(screen.getAllByTestId('agent-timeline-row')).toHaveLength(2);
    expect(screen.queryByTestId('timeline-repeat-count')).toBeNull();
  });
});

describe('ToolTimelineBlock — subagent rendering', () => {
  it('expands a subagent row even without prompt detail and shows child tool calls', () => {
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
      seq: 0,
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
      seq: 0,
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
    {
      id: 'tl-1',
      name: 'agent_prepare_context',
      round: 1,
      seq: 0,
      status: 'success',
      detail: 'fetch X',
      result: 'Prepared context from 3 sources.',
    },
    // The currently-running sub-agent (latest running).
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
    // The finished step SUCCEEDED, so its raw output is no longer duplicated
    // inline — the final answer already compresses it, and it stays reachable
    // through this row's "→". (Previously asserted present; see the
    // failure-only rule in the compact branch of ToolTimelineBlock.)
    expect(screen.queryByTestId('tool-result-output')).toBeNull();

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
            seq: 0,
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

// The in-flight viewport: while a turn is active the row list is windowed to
// a fixed height and auto-follows the newest activity, so a long run can't
// grow without bound and shove the composer around mid-turn. Settled turns
// keep their previous full-height behaviour.
describe('ToolTimelineBlock — in-flight viewport windowing', () => {
  const runningEntries: ToolTimelineEntry[] = [
    { id: 'w-1', name: 'read_file', round: 1, seq: 0, status: 'success', detail: 'a.ts' },
    { id: 'w-2', name: 'code_executor', round: 1, seq: 1, status: 'running', detail: 'run' },
  ];

  it('windows the row list while the turn is active', () => {
    renderInStore(<ToolTimelineBlock entries={runningEntries} turnActive />);
    const viewport = screen.getByTestId('tool-timeline-viewport');
    expect(viewport.getAttribute('data-windowed')).toBe('true');
    expect(viewport.className).toContain('overflow-y-auto');
  });

  it('does not window once the turn has settled', () => {
    renderInStore(<ToolTimelineBlock entries={runningEntries} turnActive={false} />);
    const viewport = screen.getByTestId('tool-timeline-viewport');
    expect(viewport.getAttribute('data-windowed')).toBe('false');
    expect(viewport.className).not.toContain('overflow-y-auto');
  });

  // Callers with no turn lifecycle to hand (settled / past-turn renders) must
  // be completely unaffected — windowing is opt-in via `turnActive`.
  it('does not window when the caller passes no turnActive', () => {
    renderInStore(<ToolTimelineBlock entries={runningEntries} />);
    expect(screen.getByTestId('tool-timeline-viewport').getAttribute('data-windowed')).toBe(
      'false'
    );
  });

  // The Agent Process Source panel wants the whole list, not a porthole.
  it('never windows under expandAllRows, even mid-turn', () => {
    renderInStore(<ToolTimelineBlock entries={runningEntries} turnActive expandAllRows />);
    expect(screen.getByTestId('tool-timeline-viewport').getAttribute('data-windowed')).toBe(
      'false'
    );
  });

  // Scrolling up detaches the auto-follow so reading an earlier step isn't
  // interrupted; returning to the bottom re-attaches it.
  it('detaches and re-attaches tail-following as the user scrolls', () => {
    renderInStore(<ToolTimelineBlock entries={runningEntries} turnActive />);
    const viewport = screen.getByTestId('tool-timeline-viewport');
    // jsdom reports 0 for all layout metrics, so drive them explicitly.
    Object.defineProperty(viewport, 'scrollHeight', { value: 500, configurable: true });
    Object.defineProperty(viewport, 'clientHeight', { value: 100, configurable: true });

    viewport.scrollTop = 0; // scrolled to the top — detached
    expect(() => fireEvent.scroll(viewport)).not.toThrow();

    viewport.scrollTop = 400; // back at the bottom — re-attached
    expect(() => fireEvent.scroll(viewport)).not.toThrow();
  });

  // The row list must not remount when a turn settles, or every <details>
  // the user opened mid-turn would snap shut.
  it('keeps the row list mounted across the settle transition', () => {
    const { rerender } = renderInStore(<ToolTimelineBlock entries={runningEntries} turnActive />);
    const before = screen.getByTestId('tool-timeline-viewport').firstElementChild;
    rerender(
      <Provider store={store}>
        <ToolTimelineBlock entries={runningEntries} turnActive={false} />
      </Provider>
    );
    const after = screen.getByTestId('tool-timeline-viewport').firstElementChild;
    expect(after).toBe(before);
  });
});

// Regression: the group used to auto-collapse in the GAP BETWEEN tools —
// `autoOpen` keyed off `isRunning` ("a tool is executing right now"), which
// goes false while the agent reasons about a result before issuing the next
// call. A just-delivered tool result appeared to be wiped a beat later, and a
// multi-tool turn flickered open/closed. The whole-turn signal (`turnActive`)
// now drives it, so the group collapses exactly once, at settle.
describe('ToolTimelineBlock — stays open between tools within a turn', () => {
  const settledRows: ToolTimelineEntry[] = [
    { id: 'g-1', name: 'read_file', round: 1, seq: 0, status: 'success', detail: 'a.ts' },
    {
      id: 'g-2',
      name: 'code_executor',
      round: 1,
      seq: 1,
      status: 'success',
      detail: 'run',
      result: 'exit 0',
    },
  ];

  it('stays open between tool calls while the turn is still active', () => {
    // No entry is `running` — the agent is reasoning before its next call.
    renderInStore(<ToolTimelineBlock entries={settledRows} turnActive />);
    expect(screen.getByTestId('agent-task-insights')).toHaveProperty('open', true);
  });

  it('collapses once the turn itself settles', () => {
    renderInStore(<ToolTimelineBlock entries={settledRows} turnActive={false} />);
    expect(screen.getByTestId('agent-task-insights')).toHaveProperty('open', false);
  });

  // Callers with no turn lifecycle fall back to `isRunning`, unchanged.
  it('falls back to isRunning when the caller passes no turnActive', () => {
    const running: ToolTimelineEntry[] = [
      { id: 'g-3', name: 'code_executor', round: 1, seq: 0, status: 'running' },
    ];
    renderInStore(<ToolTimelineBlock entries={running} />);
    expect(screen.getByTestId('agent-task-insights')).toHaveProperty('open', true);
    renderInStore(<ToolTimelineBlock entries={settledRows} />);
    expect(screen.getAllByTestId('agent-task-insights')[1]).toHaveProperty('open', false);
  });

  // The rows were never deleted — the group was merely shut. Prove the content
  // is still mounted so "wiped" can be ruled out for good.
  it('keeps the rows mounted even while collapsed', () => {
    renderInStore(<ToolTimelineBlock entries={settledRows} turnActive={false} />);
    const group = screen.getByTestId('agent-task-insights');
    expect(group).toHaveProperty('open', false);
    expect(within(group).getByTestId('tool-timeline-viewport')).toBeInTheDocument();
  });
});

// The settled-turn contract: once the final result has landed the timeline
// folds itself away so a long run never dominates the conversation, but the
// escape hatch stays reachable — "View full agent process Source →" lives in
// the always-visible <summary>, not in the collapsed body. Collapsing is only
// acceptable BECAUSE that link survives, so both halves are asserted together.
describe('ToolTimelineBlock — settled turn keeps the process-source escape hatch', () => {
  const settled: ToolTimelineEntry[] = [
    { id: 's-1', name: 'read_file', round: 1, seq: 0, status: 'success', detail: 'a.ts' },
    { id: 's-2', name: 'code_executor', round: 1, seq: 1, status: 'success', result: 'exit 0' },
  ];

  it('collapses after the final result but still exposes the process-source link', () => {
    const onViewWholeRun = vi.fn();
    renderInStore(
      <ToolTimelineBlock entries={settled} turnActive={false} onViewWholeRun={onViewWholeRun} />
    );

    const group = screen.getByTestId('agent-task-insights');
    expect(group).not.toHaveAttribute('open');

    // Link is in the <summary>, so it is reachable while collapsed.
    const link = screen.getByTestId('view-process-source');
    expect(link).toBeInTheDocument();

    // Clicking it opens the full-run panel and must NOT toggle the disclosure
    // (the handler stops propagation to the summary's own click).
    fireEvent.click(link);
    expect(onViewWholeRun).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('agent-task-insights')).not.toHaveAttribute('open');
  });

  it('still exposes the link while the turn is in flight', () => {
    const onViewWholeRun = vi.fn();
    renderInStore(
      <ToolTimelineBlock entries={settled} turnActive onViewWholeRun={onViewWholeRun} />
    );
    expect(screen.getByTestId('agent-task-insights')).toHaveAttribute('open');
    expect(screen.getByTestId('view-process-source')).toBeInTheDocument();
  });
});

// Compact chat rows show raw tool output for FAILED steps only. A successful
// step's output is already compressed into the agent's final answer, so
// repeating it inline duplicated the answer and stacked one scrollable <pre>
// per tool above it. A failure is where the answer is least trustworthy (it may
// not mention the failure at all), so that evidence stays inline.
describe('ToolTimelineBlock — compact rows show output only on failure', () => {
  const succeeded: ToolTimelineEntry = {
    id: 'r-ok',
    name: 'code_executor',
    round: 1,
    seq: 0,
    status: 'success',
    result: 'exit 0 — 42 passed',
  };
  const failed: ToolTimelineEntry = {
    id: 'r-err',
    name: 'code_executor',
    round: 1,
    seq: 1,
    status: 'error',
    result: 'exit 1 — 3 failed',
  };

  it('omits the output blob for a successful compact row', () => {
    renderInStore(<ToolTimelineBlock entries={[succeeded]} onViewDetails={vi.fn()} />);
    // Still collapsed to its link — the output is reachable, just not inline.
    expect(screen.getByTestId('view-details')).toBeInTheDocument();
    expect(screen.queryByTestId('tool-result-output')).toBeNull();
  });

  it('keeps the output blob for a failed compact row', () => {
    renderInStore(<ToolTimelineBlock entries={[failed]} onViewDetails={vi.fn()} />);
    expect(screen.getByTestId('tool-result-output').textContent).toContain('exit 1 — 3 failed');
  });

  it('shows only the failure when a turn mixes successful and failed steps', () => {
    renderInStore(<ToolTimelineBlock entries={[succeeded, failed]} onViewDetails={vi.fn()} />);
    const outputs = screen.getAllByTestId('tool-result-output');
    expect(outputs).toHaveLength(1);
    expect(outputs[0].textContent).toContain('exit 1 — 3 failed');
  });

  // The panel/expanded path is the full record and must be unaffected — a
  // successful result is still shown there.
  it('still shows successful output in the expanded/panel path', () => {
    renderInStore(<ToolTimelineBlock entries={[succeeded]} expandAllRows />);
    expect(screen.getByTestId('tool-result-output').textContent).toContain('exit 0 — 42 passed');
  });
});

// The rail renders the turn's interleaved processing transcript — narration,
// reasoning and tool steps in stream order — through the SAME
// `ProcessingTranscriptView` the Agent Process Source panel uses, so the rail
// is a windowed view of the panel rather than a second, divergent rendering.
// Narration no longer lives in the chat stream and reasoning no longer has its
// own bubble; both surface here.
describe('ToolTimelineBlock — renders the processing transcript inline', () => {
  const entries: ToolTimelineEntry[] = [
    { id: 'tx-1', name: 'web_fetch', round: 1, seq: 0, status: 'success', detail: 'example.com' },
  ];

  it('renders narration and tool steps from the transcript', () => {
    renderInStore(
      <ToolTimelineBlock
        entries={entries}
        turnActive
        transcript={[
          { kind: 'narration', round: 1, seq: 0, text: 'Let me get the data for both.' },
          { kind: 'toolCall', round: 1, seq: 1, callId: 'tx-1' },
        ]}
      />
    );
    const view = screen.getByTestId('processing-transcript');
    expect(view).toBeInTheDocument();
    expect(screen.getByTestId('processing-narration').textContent).toContain(
      'Let me get the data for both.'
    );
  });

  it('keeps the transcript inside the windowed viewport during a turn', () => {
    renderInStore(
      <ToolTimelineBlock
        entries={entries}
        turnActive
        transcript={[{ kind: 'narration', round: 1, seq: 0, text: 'Working…' }]}
      />
    );
    const viewport = screen.getByTestId('tool-timeline-viewport');
    expect(viewport.getAttribute('data-windowed')).toBe('true');
    expect(within(viewport).getByTestId('processing-transcript')).toBeInTheDocument();
  });

  // Legacy snapshots predate the transcript — those turns must still render.
  it('falls back to the tool-row list when no transcript is present', () => {
    renderInStore(<ToolTimelineBlock entries={entries} turnActive />);
    expect(screen.queryByTestId('processing-transcript')).toBeNull();
    expect(screen.getByTestId('agent-task-insights')).toBeInTheDocument();
  });

  it('falls back when the transcript is present but empty', () => {
    renderInStore(<ToolTimelineBlock entries={entries} turnActive transcript={[]} />);
    expect(screen.queryByTestId('processing-transcript')).toBeNull();
  });
});

// Regression: swapping the rail's body to `ProcessingTranscriptView` dropped
// nested sub-agent activity, because its `ToolRow` renders only title/detail/
// failure and never reads `entry.subagent`. A delegated run collapsed to one
// line and every child tool call it made became invisible — visible as the
// process-source panel (which fell back to the row list) showing more tool
// calls than the inline rail. `renderSubagent` injects the block back in;
// injected rather than imported because ToolTimelineBlock already imports
// ProcessingTranscriptView, so importing back would be a cycle.
describe('ToolTimelineBlock — sub-agent activity survives the transcript path', () => {
  const subagentEntry: ToolTimelineEntry = {
    id: 'sa-tx',
    name: 'subagent:researcher',
    round: 1,
    seq: 0,
    status: 'running',
    subagent: {
      taskId: 'task-9',
      agentId: 'researcher',
      toolCalls: [
        { callId: 'c1', toolName: 'web_search', status: 'success', elapsedMs: 120 },
        { callId: 'c2', toolName: 'web_fetch', status: 'running' },
      ],
    },
  };

  it('renders the sub-agent child tool calls inside the transcript rail', () => {
    renderInStore(
      <ToolTimelineBlock
        entries={[subagentEntry]}
        turnActive
        transcript={[{ kind: 'toolCall', round: 1, seq: 0, callId: 'sa-tx' }]}
      />
    );
    // Rendering through the transcript path…
    expect(screen.getByTestId('processing-transcript')).toBeInTheDocument();
    // …and the nested child run is present, not collapsed to one line.
    expect(screen.getByTestId('processing-subagent')).toBeInTheDocument();
    const calls = screen.getAllByTestId('subagent-tool-call');
    expect(calls).toHaveLength(2);
    expect(calls[0].textContent).toContain('Searching the web');
    expect(calls[0].textContent).toContain('Done');
    // Human label, not the raw `web_fetch` slug.
    expect(calls[1].textContent).toContain('Fetching');
    expect(calls[1].textContent).toContain('Running');
  });

  it('still renders child tool calls on the legacy row path (no transcript)', () => {
    renderInStore(<ToolTimelineBlock entries={[subagentEntry]} turnActive />);
    expect(screen.queryByTestId('processing-transcript')).toBeNull();
    expect(screen.getAllByTestId('subagent-tool-call')).toHaveLength(2);
  });

  // The nested child run must live INSIDE the windowed viewport, and must not
  // introduce a scroll container of its own. A nested scroller would clamp its
  // own height, so a streaming child run would stop changing the outer content
  // height — the ResizeObserver would never fire and auto-follow would silently
  // stall mid-subagent, with the window pinned to stale content.
  it('nests the sub-agent inside the sliding window with no scroller of its own', () => {
    renderInStore(
      <ToolTimelineBlock
        entries={[subagentEntry]}
        turnActive
        transcript={[{ kind: 'toolCall', round: 1, seq: 0, callId: 'sa-tx' }]}
      />
    );
    const viewport = screen.getByTestId('tool-timeline-viewport');
    expect(viewport.getAttribute('data-windowed')).toBe('true');

    const subagent = within(viewport).getByTestId('processing-subagent');
    expect(subagent).toBeInTheDocument();

    // Walk from the sub-agent up to the viewport: nothing between them may
    // scroll, or the outer window stops seeing the child run grow.
    for (let node = subagent; node && node !== viewport; node = node.parentElement!) {
      expect(node.className).not.toMatch(/overflow-(y-)?auto|overflow-(y-)?scroll/);
    }
  });
});

// Auto-follow: the window pins to the newest activity as the turn streams.
// jsdom ships no ResizeObserver, so the effect early-returns and this behaviour
// is invisible to every other test in this file — stub one and drive it
// directly, otherwise the single most user-visible property of the windowed
// rail has no coverage at all.
describe('ToolTimelineBlock — auto-follows the live edge', () => {
  const entries: ToolTimelineEntry[] = [
    { id: 'af-1', name: 'web_fetch', round: 1, seq: 0, status: 'running', detail: 'example.com' },
  ];
  const transcript = [
    { kind: 'narration' as const, round: 1, seq: 0, text: 'Let me get the data.' },
    { kind: 'toolCall' as const, round: 1, seq: 1, callId: 'af-1' },
  ];

  /** Installs a fake ResizeObserver and returns a trigger for its callback. */
  function stubResizeObserver() {
    const callbacks: Array<() => void> = [];
    class FakeResizeObserver {
      constructor(cb: () => void) {
        callbacks.push(cb);
      }
      observe() {}
      disconnect() {}
      unobserve() {}
    }
    (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = FakeResizeObserver;
    return {
      fire: () => callbacks.forEach(cb => cb()),
      restore: () => {
        delete (globalThis as unknown as { ResizeObserver?: unknown }).ResizeObserver;
      },
    };
  }

  /** jsdom reports 0 for all layout metrics — drive them explicitly. */
  function sizeViewport(el: HTMLElement, { scrollHeight = 600, clientHeight = 200 } = {}) {
    Object.defineProperty(el, 'scrollHeight', { value: scrollHeight, configurable: true });
    Object.defineProperty(el, 'clientHeight', { value: clientHeight, configurable: true });
  }

  it('scrolls to the newest content when the transcript grows', () => {
    const ro = stubResizeObserver();
    try {
      renderInStore(<ToolTimelineBlock entries={entries} turnActive transcript={transcript} />);
      const viewport = screen.getByTestId('tool-timeline-viewport');
      sizeViewport(viewport);
      viewport.scrollTop = 0;

      ro.fire();

      // Pinned to the live edge.
      expect(viewport.scrollTop).toBe(600);
    } finally {
      ro.restore();
    }
  });

  it('stops following once the user scrolls away from the bottom', () => {
    const ro = stubResizeObserver();
    try {
      renderInStore(<ToolTimelineBlock entries={entries} turnActive transcript={transcript} />);
      const viewport = screen.getByTestId('tool-timeline-viewport');
      sizeViewport(viewport);

      // User scrolls up to read an earlier step (well outside the 24px slack).
      viewport.scrollTop = 100;
      fireEvent.scroll(viewport);

      ro.fire();

      // Left where the user put it — not yanked back down.
      expect(viewport.scrollTop).toBe(100);
    } finally {
      ro.restore();
    }
  });

  it('resumes following when the user scrolls back to the bottom', () => {
    const ro = stubResizeObserver();
    try {
      renderInStore(<ToolTimelineBlock entries={entries} turnActive transcript={transcript} />);
      const viewport = screen.getByTestId('tool-timeline-viewport');
      sizeViewport(viewport);

      viewport.scrollTop = 100; // detach
      fireEvent.scroll(viewport);
      viewport.scrollTop = 400; // back at the bottom (600 - 200 = 400)
      fireEvent.scroll(viewport);

      ro.fire();

      expect(viewport.scrollTop).toBe(600);
    } finally {
      ro.restore();
    }
  });

  it('does not follow when the turn has settled (not windowed)', () => {
    const ro = stubResizeObserver();
    try {
      renderInStore(
        <ToolTimelineBlock entries={entries} turnActive={false} transcript={transcript} />
      );
      const viewport = screen.getByTestId('tool-timeline-viewport');
      sizeViewport(viewport);
      viewport.scrollTop = 0;

      ro.fire();

      expect(viewport.scrollTop).toBe(0);
    } finally {
      ro.restore();
    }
  });
});

// Regression: auto-follow silently never armed in a real turn.
//
// The observer used to attach in `useEffect(..., [windowed])`. `windowed` flips
// true at the START of a turn — when there is no content yet, so the component
// returned null, the ref was null, and the effect bailed. Content arriving
// afterwards re-rendered the viewport but did not change `windowed`, so the
// effect never re-ran and no observer was ever created. Every earlier test
// passed because it rendered with content already present at mount, which is
// precisely the case that never happens live.
describe('ToolTimelineBlock — auto-follow arms when content arrives after mount', () => {
  function stubResizeObserver() {
    const callbacks: Array<() => void> = [];
    class FakeResizeObserver {
      constructor(cb: () => void) {
        callbacks.push(cb);
      }
      observe() {}
      disconnect() {}
      unobserve() {}
    }
    (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = FakeResizeObserver;
    return {
      fire: () => callbacks.forEach(cb => cb()),
      count: () => callbacks.length,
      restore: () => {
        delete (globalThis as unknown as { ResizeObserver?: unknown }).ResizeObserver;
      },
    };
  }

  it('follows content that only appears on a later render', () => {
    const ro = stubResizeObserver();
    try {
      // Turn starts: windowed, but nothing to show yet → renders nothing.
      const { rerender } = renderInStore(
        <ToolTimelineBlock entries={[]} turnActive transcript={[]} />
      );
      expect(screen.queryByTestId('tool-timeline-viewport')).toBeNull();
      expect(ro.count()).toBe(0);

      // …then the first tool row lands.
      rerender(
        <Provider store={store}>
          <ToolTimelineBlock
            entries={[{ id: 'late-1', name: 'web_fetch', round: 1, seq: 0, status: 'running' }]}
            turnActive
            transcript={[]}
          />
        </Provider>
      );

      const viewport = screen.getByTestId('tool-timeline-viewport');
      Object.defineProperty(viewport, 'scrollHeight', { value: 500, configurable: true });
      Object.defineProperty(viewport, 'clientHeight', { value: 200, configurable: true });
      viewport.scrollTop = 0;

      // The observer must have been created for the node that appeared late.
      expect(ro.count()).toBeGreaterThan(0);
      ro.fire();
      expect(viewport.scrollTop).toBe(500);
    } finally {
      ro.restore();
    }
  });

  // Narration streams before the first tool call, so gating the render on
  // `entries` alone blanked the rail for the opening stretch of every turn and
  // hid tool-less turns entirely.
  it('renders on transcript alone, with no tool rows yet', () => {
    renderInStore(
      <ToolTimelineBlock
        entries={[]}
        turnActive
        transcript={[{ kind: 'narration', round: 1, seq: 0, text: 'Let me get the data.' }]}
      />
    );
    expect(screen.getByTestId('tool-timeline-viewport')).toBeInTheDocument();
    expect(screen.getByTestId('processing-narration').textContent).toContain(
      'Let me get the data.'
    );
  });

  it('still renders nothing when there is neither a row nor transcript prose', () => {
    renderInStore(<ToolTimelineBlock entries={[]} turnActive transcript={[]} />);
    expect(screen.queryByTestId('agent-task-insights')).toBeNull();
  });
});
