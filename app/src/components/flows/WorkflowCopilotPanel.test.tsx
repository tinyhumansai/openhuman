import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { WorkflowGraph, WorkflowNode } from '../../lib/flows/types';
import type { ToolTimelineEntry, WorkflowProposal } from '../../store/chatRuntimeSlice';
import WorkflowCopilotPanel from './WorkflowCopilotPanel';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

const hookState = vi.hoisted(() => ({
  sending: false,
  proposal: null as WorkflowProposal | null,
  messages: [] as Array<{ id: string; content: string; sender: 'user' | 'agent' }>,
  toolTimeline: [] as ToolTimelineEntry[],
  liveResponse: '',
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
    hookState.messages = [];
    hookState.toolTimeline = [];
    hookState.liveResponse = '';
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
    // The copilot now uses the shared ChatComposer (textarea by placeholder,
    // `send-message-button` for send).
    fireEvent.change(screen.getByPlaceholderText('flows.copilot.placeholder'), {
      target: { value: 'add a Slack notification on failure' },
    });
    fireEvent.click(screen.getByTestId('send-message-button'));

    expect(hookState.send).toHaveBeenCalledTimes(1);
    const arg = hookState.send.mock.calls[0][0];
    expect(arg.displayText).toBe('add a Slack notification on failure');
    // The brief is rendered server-side now; the panel sends a structured
    // revise request carrying the current graph as context.
    expect(arg.request.mode).toBe('revise');
    expect(arg.request.instruction).toBe('add a Slack notification on failure');
    expect(arg.request.graph).toEqual(baseGraph);
  });

  it('renders the conversation transcript (user + agent turns)', () => {
    hookState.messages = [
      { id: 'm1', content: 'add a Slack step', sender: 'user' },
      { id: 'm2', content: 'Done — proposed a Slack notification.', sender: 'agent' },
    ];
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
      />
    );
    expect(screen.getByTestId('workflow-copilot-user')).toHaveTextContent('add a Slack step');
    expect(screen.getByTestId('workflow-copilot-agent')).toHaveTextContent(
      'Done — proposed a Slack notification.'
    );
    // With a transcript present, the empty-state hint is gone.
    expect(screen.queryByTestId('workflow-copilot-empty')).not.toBeInTheDocument();
  });

  it('renders the shared tool timeline + streaming reply during a builder turn', () => {
    hookState.sending = true;
    hookState.toolTimeline = [
      { id: 'call-1', name: 'propose_workflow', round: 0, status: 'running' } as ToolTimelineEntry,
    ];
    hookState.liveResponse = 'Drafting your workflow…';
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
      />
    );
    // The shared ToolTimelineBlock renders (not the bespoke transcript), and the
    // one-shot "thinking" placeholder is suppressed once activity is streaming.
    expect(screen.getByTestId('workflow-copilot-timeline')).toBeInTheDocument();
    expect(screen.queryByTestId('workflow-copilot-thinking')).not.toBeInTheDocument();
  });

  it('shows the live reply as a bubble before the first tool call streams', () => {
    hookState.sending = true;
    hookState.toolTimeline = [];
    hookState.liveResponse = 'Thinking about your Slack digest…';
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
      />
    );
    expect(screen.getByTestId('workflow-copilot-streaming')).toHaveTextContent(
      'Thinking about your Slack digest…'
    );
    // No tool timeline yet, and the plain "thinking" line is replaced by the
    // streamed text.
    expect(screen.queryByTestId('workflow-copilot-timeline')).not.toBeInTheDocument();
    expect(screen.queryByTestId('workflow-copilot-thinking')).not.toBeInTheDocument();
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
    expect(arg.request.mode).toBe('repair');
    expect(arg.request.runId).toBe('run-7');
    expect(arg.request.error).toBe('boom');
    expect(arg.request.graph).toEqual(baseGraph);
  });

  it('auto-sends a build turn once when opened with a prompt-bar build seed', () => {
    const { rerender } = render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        flowId="flow-1"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        buildSeed={{ description: 'digest my Slack every morning' }}
      />
    );
    expect(hookState.send).toHaveBeenCalledTimes(1);
    const arg = hookState.send.mock.calls[0][0];
    // The user's description reads as their own first turn in the transcript;
    // the structured build request carries the blank graph + flow id so the
    // server's brief asks for a build → dry-run → propose arc (propose-only —
    // see #4596; persistence still waits on Accept + Save).
    expect(arg.displayText).toBe('digest my Slack every morning');
    expect(arg.request.mode).toBe('build');
    expect(arg.request.instruction).toBe('digest my Slack every morning');
    expect(arg.request.graph).toEqual(baseGraph);
    expect(arg.request.flowId).toBe('flow-1');

    // A re-render (e.g. a graph edit) must not re-fire the seed turn.
    rerender(
      <WorkflowCopilotPanel
        graph={graph(['a', 'b', 'c'])}
        flowId="flow-1"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        buildSeed={{ description: 'digest my Slack every morning' }}
      />
    );
    expect(hookState.send).toHaveBeenCalledTimes(1);
  });
});
