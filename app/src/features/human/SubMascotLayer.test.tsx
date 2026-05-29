import { render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { ToolTimelineEntry } from '../../store/chatRuntimeSlice';
import { SubMascotLayer, subMascotModelsFromTimeline } from './SubMascotLayer';

vi.mock('./Mascot', async importOriginal => {
  const actual = await importOriginal<typeof import('./Mascot')>();
  return {
    ...actual,
    RiveMascot: ({ face }: { face?: string }) => <div data-testid="rive-mascot" data-face={face} />,
  };
});

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

  it('uses child tool calls as activity for running subagents', () => {
    // subMascotModelsFromTimeline now filters to status === 'running' only,
    // so success/error entries are excluded from the rendered strip.
    const [running] = subMascotModelsFromTimeline([
      subagentEntry({
        id: 'thread-1:subagent:sub-1:code_executor',
        name: 'subagent:code_executor',
        status: 'running',
        subagent: {
          taskId: 'sub-1',
          agentId: 'code_executor',
          toolCalls: [{ callId: 'call-1', toolName: 'read_file', status: 'running' }],
        },
      }),
      // success and error entries are filtered out — only running ones appear.
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
    // success and error are filtered out — only 1 model returned.
    expect(
      subMascotModelsFromTimeline([
        subagentEntry({
          status: 'success',
          subagent: { taskId: 'sub-2', agentId: 'researcher', outputChars: 512, toolCalls: [] },
        }),
        subagentEntry({
          status: 'error',
          subagent: { taskId: 'sub-3', agentId: 'critic', toolCalls: [] },
        }),
      ])
    ).toHaveLength(0);
  });
});

describe('<SubMascotLayer />', () => {
  it('renders only running sub-mascots (success/error are filtered out)', () => {
    // The strip now only shows actively-running subagents; completed/failed
    // ones are dropped so they don't crowd the bottom of the mascot stage.
    render(
      <SubMascotLayer
        entries={[
          subagentEntry(),
          subagentEntry({
            id: 'thread-1:subagent:sub-2:planner',
            name: 'subagent:planner',
            status: 'running',
            subagent: { taskId: 'sub-2', agentId: 'planner', toolCalls: [] },
          }),
          subagentEntry({
            id: 'thread-1:subagent:sub-3:critic',
            name: 'subagent:critic',
            status: 'success',
            subagent: { taskId: 'sub-3', agentId: 'critic', outputChars: 90, toolCalls: [] },
          }),
          subagentEntry({
            id: 'thread-1:subagent:sub-4:auditor',
            name: 'subagent:auditor',
            status: 'error',
            subagent: { taskId: 'sub-4', agentId: 'auditor', toolCalls: [] },
          }),
        ]}
      />
    );

    // Only the two running entries should render.
    const mascots = screen.getAllByTestId('sub-mascot');
    expect(mascots).toHaveLength(2);
    expect(screen.getByRole('status', { name: /researcher subagent running/i })).toHaveAttribute(
      'data-status',
      'running'
    );
    expect(screen.getByRole('status', { name: /planner subagent running/i })).toHaveAttribute(
      'data-status',
      'running'
    );
    // success and error mascots are not rendered.
    expect(screen.queryByRole('status', { name: /critic subagent/i })).not.toBeInTheDocument();
    expect(screen.queryByRole('status', { name: /auditor subagent/i })).not.toBeInTheDocument();

    // Bubbles show the label text; activity is in the title attribute.
    const bubbles = screen.getAllByTestId('sub-mascot-bubble');
    expect(within(bubbles[0]!).getByText('Researcher')).toBeInTheDocument();
    expect(within(bubbles[1]!).getByText('Planner')).toBeInTheDocument();
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
