import { render, screen } from '@testing-library/react';
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

  it('renders one row per child tool call with status + timing', () => {
    renderInStore(
      <SubagentActivityBlock
        subagent={{
          taskId: 't',
          agentId: 'researcher',
          toolCalls: [
            { callId: 'c1', toolName: 'web_search', status: 'success', elapsedMs: 312 },
            { callId: 'c2', toolName: 'composio_execute', status: 'running', iteration: 2 },
            { callId: 'c3', toolName: 'noisy', status: 'error', elapsedMs: 50 },
          ],
        }}
      />
    );
    const calls = screen.getAllByTestId('subagent-tool-call');
    expect(calls).toHaveLength(3);
    expect(calls[0].textContent).toContain('web_search');
    expect(calls[0].textContent).toContain('success');
    expect(calls[0].textContent).toContain('312ms');
    expect(calls[1].textContent).toContain('running');
    expect(calls[1].textContent).toContain('·t2');
    expect(calls[2].textContent).toContain('error');
  });

  it('shows a live preview of streamed visible text (preferred over thinking)', () => {
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
    const preview = screen.getByTestId('subagent-preview');
    expect(preview.textContent).toContain('Here is what I found so far');
    // Visible text takes precedence, so the thinking tail is not shown.
    expect(preview.textContent).not.toContain('pondering');
  });

  it('falls back to the thinking tail while only reasoning has streamed', () => {
    renderInStore(
      <SubagentActivityBlock
        subagent={{
          taskId: 't',
          agentId: 'researcher',
          toolCalls: [],
          transcript: [{ kind: 'thinking', iteration: 1, text: 'I should search the web first' }],
        }}
      />
    );
    expect(screen.getByTestId('subagent-preview').textContent).toContain(
      'I should search the web first'
    );
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
    expect(calls[0].textContent).toContain('web_search');
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
