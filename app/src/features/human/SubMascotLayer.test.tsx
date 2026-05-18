import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { ToolTimelineEntry } from '../../store/chatRuntimeSlice';
import { SubMascotLayer, subMascotModelsFromTimeline } from './SubMascotLayer';

function subagentEntry(overrides: Partial<ToolTimelineEntry> = {}): ToolTimelineEntry {
  return {
    id: 'thread-1:subagent:sub-1:researcher',
    name: 'subagent:researcher',
    round: 1,
    status: 'running',
    detail: 'Research the relevant docs.',
    subagent: {
      taskId: 'sub-1',
      agentId: 'researcher',
      childIteration: 1,
      childMaxIterations: 4,
      toolCalls: [],
    },
    ...overrides,
  };
}

describe('subMascotModelsFromTimeline', () => {
  it('builds visible models only from subagent timeline rows', () => {
    const models = subMascotModelsFromTimeline([
      { id: 'thread-1:tool:search', name: 'web_search', round: 1, status: 'running' },
      subagentEntry(),
    ]);

    expect(models).toHaveLength(1);
    expect(models[0]).toMatchObject({
      agentId: 'researcher',
      label: 'Researcher',
      status: 'running',
      face: 'thinking',
      activity: 'Iteration 1/4',
    });
  });

  it('uses child tool calls, completion, and failure as activity bubbles', () => {
    const [running, success, error] = subMascotModelsFromTimeline([
      subagentEntry({
        id: 'thread-1:subagent:sub-1:code_executor',
        name: 'subagent:code_executor',
        subagent: {
          taskId: 'sub-1',
          agentId: 'code_executor',
          toolCalls: [{ callId: 'call-1', toolName: 'read_file', status: 'running' }],
        },
      }),
      subagentEntry({
        id: 'thread-1:subagent:sub-2:researcher',
        status: 'success',
        subagent: { taskId: 'sub-2', agentId: 'researcher', outputChars: 512, toolCalls: [] },
      }),
      subagentEntry({
        id: 'thread-1:subagent:sub-3:critic',
        name: 'subagent:critic',
        status: 'error',
        subagent: { taskId: 'sub-3', agentId: 'critic', toolCalls: [] },
      }),
    ]);

    expect(running?.activity).toBe('Using Read File');
    expect(running?.face).toBe('thinking');
    expect(success?.activity).toBe('Completed 512 chars');
    expect(success?.face).toBe('happy');
    expect(error?.activity).toBe('Needs attention');
    expect(error?.face).toBe('concerned');
  });
});

describe('<SubMascotLayer />', () => {
  it('renders multiple colored sub-mascots with running, success, and failed states', () => {
    render(
      <SubMascotLayer
        entries={[
          subagentEntry(),
          subagentEntry({
            id: 'thread-1:subagent:sub-2:planner',
            name: 'subagent:planner',
            status: 'success',
            subagent: { taskId: 'sub-2', agentId: 'planner', outputChars: 90, toolCalls: [] },
          }),
          subagentEntry({
            id: 'thread-1:subagent:sub-3:critic',
            name: 'subagent:critic',
            status: 'error',
            subagent: { taskId: 'sub-3', agentId: 'critic', toolCalls: [] },
          }),
        ]}
      />
    );

    const mascots = screen.getAllByTestId('sub-mascot');
    expect(mascots).toHaveLength(3);
    expect(screen.getByRole('status', { name: /researcher subagent running/i })).toHaveAttribute(
      'data-status',
      'running'
    );
    expect(screen.getByRole('status', { name: /planner subagent success/i })).toHaveAttribute(
      'data-status',
      'success'
    );
    expect(screen.getByRole('status', { name: /critic subagent error/i })).toHaveAttribute(
      'data-status',
      'error'
    );

    const bubbles = screen.getAllByTestId('sub-mascot-bubble');
    expect(within(bubbles[0]!).getByText('Researcher')).toBeInTheDocument();
    expect(within(bubbles[1]!).getByText('Completed 90 chars')).toBeInTheDocument();
    expect(within(bubbles[2]!).getByText('Needs attention')).toBeInTheDocument();
  });

  it('renders nothing when no subagent rows are present', () => {
    const { container } = render(
      <SubMascotLayer
        entries={[{ id: 'tool-1', name: 'web_search', round: 1, status: 'running' }]}
      />
    );

    expect(container).toBeEmptyDOMElement();
  });
});
