import { render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it } from 'vitest';

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
