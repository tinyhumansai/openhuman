import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { WorkflowGraph, WorkflowNode } from '../../lib/flows/types';
import type { WorkflowProposal } from '../../store/chatRuntimeSlice';
import WorkflowCopilotPanel from './WorkflowCopilotPanel';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

const hookState = vi.hoisted(() => ({
  sending: false,
  proposal: null as WorkflowProposal | null,
  error: null as string | null,
  send: vi.fn(),
  clearProposal: vi.fn(),
}));
vi.mock('../../hooks/useWorkflowBuilderChat', () => ({ useWorkflowBuilderChat: () => hookState }));

function node(id: string): WorkflowNode {
  return { id, kind: 'agent', name: id, config: {}, ports: [] };
}
function graph(ids: string[]): WorkflowGraph {
  return { schema_version: 1, name: 'g', nodes: ids.map(node), edges: [] };
}

function proposalWith(ids: string[]): WorkflowProposal {
  return {
    name: 'Revised flow',
    graph: graph(ids),
    requireApproval: true,
    summary: { trigger: 'manual', steps: [] },
  };
}

const baseGraph = graph(['a', 'b']);

describe('WorkflowCopilotPanel', () => {
  beforeEach(() => {
    hookState.sending = false;
    hookState.proposal = null;
    hookState.error = null;
    hookState.send = vi.fn().mockResolvedValue(undefined);
    hookState.clearProposal = vi.fn();
  });

  it('sends a revise turn that injects the current graph', async () => {
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
      />
    );
    fireEvent.change(screen.getByTestId('workflow-copilot-input'), {
      target: { value: 'add a Slack notification on failure' },
    });
    fireEvent.click(screen.getByTestId('workflow-copilot-send'));

    expect(hookState.send).toHaveBeenCalledTimes(1);
    const arg = hookState.send.mock.calls[0][0];
    expect(arg.displayText).toBe('add a Slack notification on failure');
    expect(arg.prompt).toContain(JSON.stringify(baseGraph));
  });

  it('surfaces a new proposal to the host and shows the added/removed diff', () => {
    const onProposal = vi.fn();
    // proposed drops "b" and adds "c" vs. base [a, b].
    hookState.proposal = proposalWith(['a', 'c']);
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        onProposal={onProposal}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
      />
    );
    expect(onProposal).toHaveBeenCalledWith(hookState.proposal);
    // Both a single added ("c") and a single removed ("b") badge appear.
    expect(screen.getByTestId('workflow-copilot-added')).toBeInTheDocument();
    expect(screen.getByTestId('workflow-copilot-removed')).toBeInTheDocument();
  });

  it('Accept applies to the draft and clears the proposal (never persists)', () => {
    const onAccept = vi.fn();
    hookState.proposal = proposalWith(['a', 'c']);
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        onProposal={vi.fn()}
        onAccept={onAccept}
        onReject={vi.fn()}
        onClose={vi.fn()}
      />
    );
    fireEvent.click(screen.getByTestId('workflow-copilot-accept'));
    expect(onAccept).toHaveBeenCalledWith(hookState.proposal);
    expect(hookState.clearProposal).toHaveBeenCalledTimes(1);
  });

  it('Reject discards the proposal without applying it', () => {
    const onReject = vi.fn();
    const onAccept = vi.fn();
    hookState.proposal = proposalWith(['a', 'c']);
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        onProposal={vi.fn()}
        onAccept={onAccept}
        onReject={onReject}
        onClose={vi.fn()}
      />
    );
    fireEvent.click(screen.getByTestId('workflow-copilot-reject'));
    expect(onReject).toHaveBeenCalledTimes(1);
    expect(onAccept).not.toHaveBeenCalled();
    expect(hookState.clearProposal).toHaveBeenCalledTimes(1);
  });

  it('auto-sends a repair turn once when opened with a repair seed', () => {
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        repairSeed={{ runId: 'run-7', error: 'boom', graph: baseGraph }}
      />
    );
    expect(hookState.send).toHaveBeenCalledTimes(1);
    const arg = hookState.send.mock.calls[0][0];
    expect(arg.prompt).toContain('run-7');
    expect(arg.prompt).toContain('get_flow_run');
  });
});
