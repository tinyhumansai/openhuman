import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { WorkflowGraph, WorkflowNode } from '../../lib/flows/types';
import type { ToolTimelineEntry, WorkflowProposal } from '../../store/chatRuntimeSlice';
import WorkflowCopilotPanel from './WorkflowCopilotPanel';

vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

interface MockMessage {
  id: string;
  content: string;
  sender: 'user' | 'agent';
  extraMetadata?: { isInterim?: boolean };
}

const hookState = vi.hoisted(() => ({
  sending: false,
  proposal: null as WorkflowProposal | null,
  capped: false,
  // Panel renders `displayMessages` (already interim-filtered upstream by
  // `useWorkflowBuilderChat`) — kept separate from `messages` in these tests
  // so a mismatch between the two proves the panel is reading the right field.
  displayMessages: [] as MockMessage[],
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
    hookState.capped = false;
    hookState.displayMessages = [];
    hookState.toolTimeline = [];
    hookState.liveResponse = '';
    hookState.error = null;
    hookState.send = vi.fn().mockResolvedValue({ outcome: 'dispatched', proposed: false });
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

  it('carries the original ask forward across a clarifying-question turn, then drops it once a proposal lands', async () => {
    hookState.send = vi
      .fn()
      // Turn 1: the agent asks a clarifying question instead of proposing.
      .mockResolvedValueOnce({ proposed: false })
      // Turn 2: the user's answer resolves it and a proposal lands.
      .mockResolvedValueOnce({ proposed: true })
      // Turn 3 (and any further calls): a normal revise turn, already resolved.
      .mockResolvedValue({ proposed: true });

    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
      />
    );

    fireEvent.change(screen.getByPlaceholderText('flows.copilot.placeholder'), {
      target: { value: 'post a daily summary to slack' },
    });
    await act(async () => {
      fireEvent.click(screen.getByTestId('send-message-button'));
      // Flush the microtasks `submit` awaits before it records `pendingAskRef`.
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(hookState.send).toHaveBeenCalledTimes(1);

    fireEvent.change(screen.getByPlaceholderText('flows.copilot.placeholder'), {
      target: { value: '#eng' },
    });
    await act(async () => {
      fireEvent.click(screen.getByTestId('send-message-button'));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(hookState.send).toHaveBeenCalledTimes(2);
    const secondArg = hookState.send.mock.calls[1][0];
    // The follow-up must carry the ORIGINAL ask forward — a bare "#eng" alone
    // would strand the agent with no idea what it was asked to build (the
    // current graph is still blank/unchanged since no proposal has landed).
    expect(secondArg.request.mode).toBe('revise');
    expect(secondArg.request.instruction).toContain('post a daily summary to slack');
    expect(secondArg.request.instruction).toContain('#eng');

    // Turn 3, after a proposal has landed: the graph itself now carries the
    // state, so the original ask must NOT be repeated.
    fireEvent.change(screen.getByPlaceholderText('flows.copilot.placeholder'), {
      target: { value: 'also add a filter step' },
    });
    fireEvent.click(screen.getByTestId('send-message-button'));
    expect(hookState.send).toHaveBeenCalledTimes(3);
    const thirdArg = hookState.send.mock.calls[2][0];
    expect(thirdArg.request.instruction).toBe('also add a filter step');
  });

  it('renders the conversation transcript (user + agent turns)', () => {
    hookState.displayMessages = [
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

  it('B25: unwraps a raw tool-call envelope message into clean text + a tool activity chip, never raw JSON', () => {
    // Repro for B25: a turn that both talks and calls a tool can land in the
    // thread transcript as the provider wire-format `{ content, tool_calls }`
    // envelope. The panel must render only the human text — never the raw
    // JSON — plus a compact status chip for the tool activity.
    hookState.displayMessages = [
      { id: 'm1', content: 'build me a Slack digest', sender: 'user' },
      {
        id: 'm2',
        content: JSON.stringify({
          content: "Here's the workflow I propose.",
          tool_calls: [{ id: 'call_1', name: 'propose_workflow', arguments: '{"nodes":[]}' }],
        }),
        sender: 'agent',
      },
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
    const bubble = screen.getByTestId('workflow-copilot-agent');
    expect(bubble).toHaveTextContent("Here's the workflow I propose.");
    // The raw envelope must never reach the DOM as text.
    expect(bubble).not.toHaveTextContent('tool_calls');
    expect(bubble).not.toHaveTextContent('"nodes":[]');
    expect(screen.getByTestId('tool-activity-chip')).toHaveTextContent(
      'flows.copilot.tool.proposing'
    );
  });

  it('does not render an isInterim agent message as a bubble, only the terminal one', () => {
    // The panel only ever renders `displayMessages` — the same filtered set
    // `useWorkflowBuilderChat` computes from the raw transcript (isInterim
    // agent messages dropped since that narration already streams live via
    // the tool timeline / live text). Mirror that filter here so this test
    // documents (and would catch a regression in) the composition: an
    // isInterim message must never reach the panel as a bubble, while the
    // terminal (non-interim) answer still does.
    const raw: MockMessage[] = [
      { id: 'm1', content: 'build me a Slack digest', sender: 'user' },
      {
        id: 'm2',
        content: 'Let me check your calendar first.',
        sender: 'agent',
        extraMetadata: { isInterim: true },
      },
      { id: 'm3', content: 'Done — proposed a Slack notification.', sender: 'agent' },
    ];
    hookState.displayMessages = raw.filter(m => m.sender === 'user' || !m.extraMetadata?.isInterim);

    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
      />
    );
    expect(screen.queryByText('Let me check your calendar first.')).not.toBeInTheDocument();
    expect(screen.getByTestId('workflow-copilot-agent')).toHaveTextContent(
      'Done — proposed a Slack notification.'
    );
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

  it('Accept calls onAccept (host applies + saves) and clears the proposal once it resolves', async () => {
    const onAccept = vi.fn().mockResolvedValue(undefined);
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
    await waitFor(() => expect(hookState.clearProposal).toHaveBeenCalledTimes(1));
  });

  it('shows the saving label and disables Accept while the host save is in flight', async () => {
    // Deferred promise so the test controls exactly when the host's save
    // (`onAccept`) resolves, to observe the in-between "saving" state.
    let resolveSave!: () => void;
    const savePromise = new Promise<void>(resolve => {
      resolveSave = resolve;
    });
    const onAccept = vi.fn().mockReturnValue(savePromise);
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
    await waitFor(() =>
      expect(screen.getByTestId('workflow-copilot-accept')).toHaveTextContent(
        'flows.copilot.saving'
      )
    );
    expect(screen.getByTestId('workflow-copilot-accept')).toBeDisabled();
    expect(hookState.clearProposal).not.toHaveBeenCalled();

    resolveSave();
    await waitFor(() => expect(hookState.clearProposal).toHaveBeenCalledTimes(1));
  });

  it('leaves the proposal visible for retry when the host save rejects', async () => {
    const onAccept = vi.fn().mockRejectedValue(new Error('save failed'));
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
    await waitFor(() => expect(onAccept).toHaveBeenCalledTimes(1));
    // The button re-enables once the rejected save settles, and the proposal
    // was never cleared — the card stays up so the user can retry.
    await waitFor(() => expect(screen.getByTestId('workflow-copilot-accept')).not.toBeDisabled());
    expect(hookState.clearProposal).not.toHaveBeenCalled();
  });

  it('disables Reject while an Accept save is in flight, so it cannot race the persisted save', async () => {
    // Regression for the CodeRabbit finding: Reject must not stay clickable
    // while `onAccept`'s save is still pending, otherwise the user's cancel
    // can be silently overridden by the earlier Accept's save landing after.
    let resolveSave!: () => void;
    const savePromise = new Promise<void>(resolve => {
      resolveSave = resolve;
    });
    const onAccept = vi.fn().mockReturnValue(savePromise);
    const onReject = vi.fn();
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

    fireEvent.click(screen.getByTestId('workflow-copilot-accept'));
    await waitFor(() => expect(screen.getByTestId('workflow-copilot-reject')).toBeDisabled());

    // A click while disabled is a no-op in jsdom/RTL — Reject must not fire.
    fireEvent.click(screen.getByTestId('workflow-copilot-reject'));
    expect(onReject).not.toHaveBeenCalled();
    expect(hookState.clearProposal).not.toHaveBeenCalled();

    resolveSave();
    await waitFor(() => expect(hookState.clearProposal).toHaveBeenCalledTimes(1));
    expect(screen.getByTestId('workflow-copilot-reject')).not.toBeDisabled();
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

  it('B34: renders a "Continue building" card when the turn hit the iteration cap', () => {
    hookState.capped = true;
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
      />
    );
    expect(screen.getByTestId('workflow-copilot-capped')).toBeInTheDocument();
    expect(screen.getByTestId('workflow-copilot-continue')).toBeInTheDocument();
  });

  it('B34: does NOT render the capped card for a normal (non-capped) turn', () => {
    hookState.capped = false;
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
      />
    );
    expect(screen.queryByTestId('workflow-copilot-capped')).not.toBeInTheDocument();
  });

  it('B34: does not render the capped card while a proposal is pending, even if capped is stale-true', () => {
    // Defense-in-depth: the server already scopes `capped` to `proposal ===
    // null`, but the panel re-checks `!proposal` itself too (see the JSX
    // condition) in case a stale `capped=true` from a prior turn outlives a
    // later turn's proposal.
    hookState.capped = true;
    hookState.proposal = proposalWith(['a', 'c']);
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
      />
    );
    expect(screen.queryByTestId('workflow-copilot-capped')).not.toBeInTheDocument();
  });

  it('B34: clicking "Continue building" sends a follow-up turn', async () => {
    hookState.capped = true;
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
      />
    );
    fireEvent.click(screen.getByTestId('workflow-copilot-continue'));
    await waitFor(() => expect(hookState.send).toHaveBeenCalledTimes(1));
    const arg = hookState.send.mock.calls[0][0];
    expect(arg.request.mode).toBe('revise');
    expect(arg.request.graph).toEqual(baseGraph);
  });

  // Codex review on #4865: "Continue building" must resume ON the current
  // draft — a `revise` turn over the EXISTING `flowId`, never a blank/`create`
  // restart — since `flows_build` spins up a fresh `workflow_builder` agent
  // per RPC with no server-side session/checkpoint to resume. Carrying the
  // live `graph` + `flowId` is what makes "Continue" a correct, working
  // continuation instead of an empty restart.
  it('B34: "Continue building" carries the current flowId, not a blank restart', async () => {
    hookState.capped = true;
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        flowId="flow-123"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
      />
    );
    fireEvent.click(screen.getByTestId('workflow-copilot-continue'));
    await waitFor(() => expect(hookState.send).toHaveBeenCalledTimes(1));
    const arg = hookState.send.mock.calls[0][0];
    expect(arg.request.mode).toBe('revise');
    expect(arg.request.flowId).toBe('flow-123');
    expect(arg.request.graph).toEqual(baseGraph);
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

  it('reports the build seed as consumed once the turn actually dispatched', async () => {
    const onBuildSeedConsumed = vi.fn();
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        flowId="flow-1"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        buildSeed={{ description: 'digest my Slack every morning' }}
        onBuildSeedConsumed={onBuildSeedConsumed}
      />
    );
    expect(hookState.send).toHaveBeenCalledTimes(1);
    // Fires exactly once, after the dispatched build turn resolves, so the
    // route seed can be stripped.
    await waitFor(() => expect(onBuildSeedConsumed).toHaveBeenCalledTimes(1));
  });

  it('does not consume the seed when send no-ops (socket not connected) (#4597)', async () => {
    // `send` resolves `outcome: 'skipped'` when the socket isn't connected —
    // the turn never dispatched, so the seed must be preserved (not cleared) so
    // the build can still fire once the socket connects.
    hookState.send = vi.fn().mockResolvedValue({ outcome: 'skipped', proposed: false });
    const onBuildSeedConsumed = vi.fn();
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        flowId="flow-1"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        buildSeed={{ description: 'digest my Slack every morning' }}
        onBuildSeedConsumed={onBuildSeedConsumed}
      />
    );
    expect(hookState.send).toHaveBeenCalledTimes(1);
    // Flush the actual send() promise so the effect's `.then` runs, then assert
    // the no-op path never consumed the seed.
    await hookState.send.mock.results[0]?.value;
    await waitFor(() => expect(onBuildSeedConsumed).not.toHaveBeenCalled());
  });

  it('does not consume or resend the seed when send fails (#4597)', async () => {
    // A dispatch error resolves 'failed' (not 'skipped'): the seed is NOT
    // consumed, and — crucially — the guard stays set so the effect does not
    // auto-resend the turn (which would duplicate the user message and hammer
    // the backend). The error surfaces separately for the user to retry.
    hookState.send = vi.fn().mockResolvedValue({ outcome: 'failed', proposed: false });
    const onBuildSeedConsumed = vi.fn();
    const { rerender } = render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        flowId="flow-1"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        buildSeed={{ description: 'digest my Slack every morning' }}
        onBuildSeedConsumed={onBuildSeedConsumed}
      />
    );
    expect(hookState.send).toHaveBeenCalledTimes(1);
    await hookState.send.mock.results[0]?.value;
    expect(onBuildSeedConsumed).not.toHaveBeenCalled();

    // A re-render with a fresh `send` identity (as happens on any state change)
    // must NOT resend — the guard remains set for a failed dispatch.
    hookState.send = vi.fn().mockResolvedValue({ outcome: 'failed', proposed: false });
    rerender(
      <WorkflowCopilotPanel
        graph={baseGraph}
        flowId="flow-1"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        buildSeed={{ description: 'digest my Slack every morning' }}
        onBuildSeedConsumed={onBuildSeedConsumed}
      />
    );
    expect(hookState.send).not.toHaveBeenCalled();
    expect(onBuildSeedConsumed).not.toHaveBeenCalled();
  });

  it('does not re-fire the build turn when remounted after the seed is cleared (#4597)', async () => {
    const onBuildSeedConsumed = vi.fn();
    // First mount with the seed present (as the prompt-bar route lands).
    const { unmount } = render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        flowId="flow-1"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        buildSeed={{ description: 'digest my Slack every morning' }}
        onBuildSeedConsumed={onBuildSeedConsumed}
      />
    );
    expect(hookState.send).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(onBuildSeedConsumed).toHaveBeenCalledTimes(1));

    // The host clears the route seed (buildSeed -> null) in response. Closing
    // and reopening the copilot fully remounts the panel — the per-mount
    // `buildSentRef` resets to false — but with no seed there is nothing to
    // re-fire, so the build turn must NOT be sent a second time.
    unmount();
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        flowId="flow-1"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        buildSeed={null}
        onBuildSeedConsumed={onBuildSeedConsumed}
      />
    );
    expect(hookState.send).toHaveBeenCalledTimes(1);
    expect(onBuildSeedConsumed).toHaveBeenCalledTimes(1);
  });

  it('carries the build seed description forward when the auto-sent build turn asks a clarifying question instead of proposing', async () => {
    hookState.send = vi
      .fn()
      // The auto-sent build turn dispatches but asks a question rather than
      // proposing.
      .mockResolvedValueOnce({ outcome: 'dispatched', proposed: false })
      // The user's free-text answer then resolves it.
      .mockResolvedValueOnce({ outcome: 'dispatched', proposed: true });

    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        flowId="flow-1"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        buildSeed={{ description: 'post a daily summary to slack' }}
      />
    );
    // Flush the microtasks the seed effect awaits before recording
    // `pendingAskRef` from the resolved `{ proposed: false }`.
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(hookState.send).toHaveBeenCalledTimes(1);

    fireEvent.change(screen.getByPlaceholderText('flows.copilot.placeholder'), {
      target: { value: '#eng' },
    });
    fireEvent.click(screen.getByTestId('send-message-button'));

    expect(hookState.send).toHaveBeenCalledTimes(2);
    const secondArg = hookState.send.mock.calls[1][0];
    // The follow-up must carry the build seed's original description forward,
    // not just the bare "#eng" answer.
    expect(secondArg.request.instruction).toContain('post a daily summary to slack');
    expect(secondArg.request.instruction).toContain('#eng');
  });

  it('populates the composer input from a prefill seed WITHOUT sending it', () => {
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        flowId="flow-1"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        prefillSeed={{ text: 'Build a workflow that files receipts.' }}
      />
    );
    // The Suggested Workflows "Build this" prefill never auto-sends — only
    // populates the input so the user can review/edit before pressing Send.
    expect(hookState.send).not.toHaveBeenCalled();
    expect(screen.getByPlaceholderText('flows.copilot.placeholder')).toHaveValue(
      'Build a workflow that files receipts.'
    );
  });

  it('reports the prefill seed as consumed once applied, so the host can clear the route seed', () => {
    const onPrefillSeedConsumed = vi.fn();
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        flowId="flow-1"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        prefillSeed={{ text: 'Build a workflow that files receipts.' }}
        onPrefillSeedConsumed={onPrefillSeedConsumed}
      />
    );
    expect(onPrefillSeedConsumed).toHaveBeenCalledTimes(1);
  });

  it('does not re-apply the prefill seed on a re-render (would clobber in-progress edits)', () => {
    const onPrefillSeedConsumed = vi.fn();
    const { rerender } = render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        flowId="flow-1"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        prefillSeed={{ text: 'Build a workflow that files receipts.' }}
        onPrefillSeedConsumed={onPrefillSeedConsumed}
      />
    );
    const input = screen.getByPlaceholderText('flows.copilot.placeholder');
    expect(input).toHaveValue('Build a workflow that files receipts.');

    // The user edits the pre-filled text.
    fireEvent.change(input, { target: { value: 'Build a workflow that files receipts weekly.' } });

    // A re-render (e.g. a graph edit) with the same seed must not re-apply it
    // and clobber the user's in-progress edit.
    rerender(
      <WorkflowCopilotPanel
        graph={graph(['a', 'b', 'c'])}
        flowId="flow-1"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        prefillSeed={{ text: 'Build a workflow that files receipts.' }}
        onPrefillSeedConsumed={onPrefillSeedConsumed}
      />
    );
    expect(screen.getByPlaceholderText('flows.copilot.placeholder')).toHaveValue(
      'Build a workflow that files receipts weekly.'
    );
    expect(onPrefillSeedConsumed).toHaveBeenCalledTimes(1);
    expect(hookState.send).not.toHaveBeenCalled();
  });

  it('does not re-apply the prefill seed when remounted after the seed is cleared', () => {
    const onPrefillSeedConsumed = vi.fn();
    const { unmount } = render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        flowId="flow-1"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        prefillSeed={{ text: 'Build a workflow that files receipts.' }}
        onPrefillSeedConsumed={onPrefillSeedConsumed}
      />
    );
    expect(onPrefillSeedConsumed).toHaveBeenCalledTimes(1);

    // The host clears the route seed (prefillSeed -> null) in response.
    // Closing and reopening the copilot fully remounts it; with no seed left
    // there is nothing to re-apply.
    unmount();
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        flowId="flow-1"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        prefillSeed={null}
        onPrefillSeedConsumed={onPrefillSeedConsumed}
      />
    );
    expect(onPrefillSeedConsumed).toHaveBeenCalledTimes(1);
    expect(hookState.send).not.toHaveBeenCalled();
  });

  it("sends the FIRST Send after a prefill seed with the seed's builder mode, not revise", async () => {
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        flowId="flow-1"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        prefillSeed={{ text: 'Build a workflow that files receipts.', mode: 'build' }}
      />
    );

    fireEvent.click(screen.getByTestId('send-message-button'));

    expect(hookState.send).toHaveBeenCalledTimes(1);
    const arg = hookState.send.mock.calls[0][0];
    // First Send after a Suggested Workflows prefill must run the seed's
    // `build` mode (build → dry-run → propose against the just-created blank
    // flow) — NOT the panel's usual `revise` turn.
    expect(arg.request.mode).toBe('build');
    expect(arg.request.instruction).toBe('Build a workflow that files receipts.');
    expect(arg.request.flowId).toBe('flow-1');
  });

  it('falls back to revise for subsequent Sends after the prefill-seeded first one', async () => {
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        flowId="flow-1"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        prefillSeed={{ text: 'Build a workflow that files receipts.', mode: 'build' }}
      />
    );

    fireEvent.click(screen.getByTestId('send-message-button'));
    expect(hookState.send.mock.calls[0][0].request.mode).toBe('build');

    fireEvent.change(screen.getByPlaceholderText('flows.copilot.placeholder'), {
      target: { value: 'also add a retry' },
    });
    fireEvent.click(screen.getByTestId('send-message-button'));

    expect(hookState.send).toHaveBeenCalledTimes(2);
    expect(hookState.send.mock.calls[1][0].request.mode).toBe('revise');
  });

  it('defaults an omitted prefill seed mode to build on the first Send', async () => {
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        flowId="flow-1"
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        prefillSeed={{ text: 'Build a workflow that files receipts.' }}
      />
    );

    fireEvent.click(screen.getByTestId('send-message-button'));

    expect(hookState.send.mock.calls[0][0].request.mode).toBe('build');
  });

  it('falls back to revise on the first Send when there is no flow id to build against', async () => {
    render(
      <WorkflowCopilotPanel
        graph={baseGraph}
        onProposal={vi.fn()}
        onAccept={vi.fn()}
        onReject={vi.fn()}
        onClose={vi.fn()}
        prefillSeed={{ text: 'Build a workflow that files receipts.', mode: 'build' }}
      />
    );

    fireEvent.click(screen.getByTestId('send-message-button'));

    expect(hookState.send.mock.calls[0][0].request.mode).toBe('revise');
  });
});
