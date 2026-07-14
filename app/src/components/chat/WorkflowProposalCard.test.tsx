import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { WorkflowProposal } from '../../store/chatRuntimeSlice';
import { WorkflowProposalCard } from './WorkflowProposalCard';

// Echo i18n keys so we can assert on the stable key string.
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

// `vi.mock` factories are hoisted above the module's top-level statements, so
// every handle a factory closes over must be declared via `vi.hoisted` rather
// than a plain top-level `const` — otherwise it'd be a TDZ reference at the
// time the (hoisted) factory runs. (These specific names happened to work
// without it, since Vitest's compiler special-cases `mock`-prefixed
// identifiers, but that's an incidental heuristic, not a guarantee.)
const { mockCreateFlow, mockUpdateFlow, mockSetFlowEnabled, mockDispatch, mockNavigate } =
  vi.hoisted(() => ({
    mockCreateFlow: vi.fn(),
    mockUpdateFlow: vi.fn(),
    mockSetFlowEnabled: vi.fn(),
    mockDispatch: vi.fn(),
    mockNavigate: vi.fn(),
  }));
vi.mock('../../services/api/flowsApi', () => ({
  createFlow: (...args: unknown[]) => mockCreateFlow(...args),
  updateFlow: (...args: unknown[]) => mockUpdateFlow(...args),
  setFlowEnabled: (...args: unknown[]) => mockSetFlowEnabled(...args),
}));
vi.mock('../../store/hooks', () => ({ useAppDispatch: () => mockDispatch }));
vi.mock('react-router-dom', () => ({ useNavigate: () => mockNavigate }));

function proposal(partial: Partial<WorkflowProposal> = {}): WorkflowProposal {
  return {
    name: 'Daily standup summary',
    graph: { nodes: [], edges: [] },
    requireApproval: true,
    summary: {
      trigger: 'schedule: 0 9 * * *',
      steps: [
        { kind: 'agent', name: 'Summarize', config_hint: "Summarize yesterday's messages" },
        { kind: 'tool_call', name: 'Post to Slack' },
      ],
    },
    ...partial,
  };
}

describe('WorkflowProposalCard', () => {
  beforeEach(() => {
    mockCreateFlow
      .mockReset()
      .mockResolvedValue({ id: 'f1', name: 'Daily standup summary', enabled: true });
    mockUpdateFlow.mockReset();
    mockSetFlowEnabled.mockReset().mockResolvedValue({ id: 'f1', enabled: true });
    mockDispatch.mockReset();
    mockNavigate.mockReset();
  });

  it('renders the name, trigger, and steps with node-kind badges', () => {
    render(<WorkflowProposalCard threadId="t1" proposal={proposal()} />);
    expect(screen.getByText('Daily standup summary')).toBeInTheDocument();
    expect(screen.getByText('schedule: 0 9 * * *')).toBeInTheDocument();
    expect(screen.getByText('Summarize')).toBeInTheDocument();
    expect(screen.getByText('Post to Slack')).toBeInTheDocument();
    expect(screen.getByText('agent')).toBeInTheDocument();
    expect(screen.getByText('tool_call')).toBeInTheDocument();
    expect(screen.getAllByTestId('workflow-proposal-step-kind')).toHaveLength(2);
  });

  it('has the expected root test id', () => {
    render(<WorkflowProposalCard threadId="t1" proposal={proposal()} />);
    expect(screen.getByTestId('workflow-proposal-card')).toBeInTheDocument();
  });

  it('saves via createFlow with the right args and clears optimistically', async () => {
    const p = proposal();
    render(<WorkflowProposalCard threadId="t1" proposal={p} />);
    fireEvent.click(screen.getByText('chat.flowProposal.save'));
    await waitFor(() =>
      expect(mockCreateFlow).toHaveBeenCalledWith(p.name, p.graph, p.requireApproval)
    );
    expect(mockDispatch).toHaveBeenCalledTimes(1);
    // createFlow already came back enabled — no need for a follow-up arm.
    expect(mockSetFlowEnabled).not.toHaveBeenCalled();
  });

  it('shows a loading state while saving', async () => {
    let resolveCreate!: (value: unknown) => void;
    mockCreateFlow.mockReturnValueOnce(
      new Promise(resolve => {
        resolveCreate = resolve;
      })
    );
    render(<WorkflowProposalCard threadId="t1" proposal={proposal()} />);
    fireEvent.click(screen.getByText('chat.flowProposal.save'));
    await waitFor(() => expect(screen.getByText('chat.flowProposal.saving')).toBeInTheDocument());
    resolveCreate({ id: 'f1', enabled: true });
  });

  it('surfaces an error and stays mounted when createFlow fails', async () => {
    mockCreateFlow.mockRejectedValueOnce(new Error('boom'));
    render(<WorkflowProposalCard threadId="t1" proposal={proposal()} />);
    fireEvent.click(screen.getByText('chat.flowProposal.save'));
    await waitFor(() => expect(screen.getByText(/chat\.flowProposal\.error/)).toBeInTheDocument());
    // Not cleared on failure.
    expect(mockDispatch).not.toHaveBeenCalled();
    expect(mockSetFlowEnabled).not.toHaveBeenCalled();
  });

  // Issue B29 Rule 1: an automatic-trigger graph (schedule/app_event/webhook)
  // comes back from `flows_create` disabled, regardless of what the caller
  // asked for. "Save & enable" is the user's own explicit arming click, so
  // the card must follow up with `setFlowEnabled` to actually arm it —
  // otherwise the CTA's own label would be a lie and the flow would never
  // fire despite the user clicking "enable".
  it('explicitly enables via setFlowEnabled when createFlow returns a disabled auto-trigger flow', async () => {
    mockCreateFlow.mockResolvedValueOnce({
      id: 'f1',
      name: 'Daily standup summary',
      enabled: false,
    });
    const p = proposal();
    render(<WorkflowProposalCard threadId="t1" proposal={p} />);
    fireEvent.click(screen.getByText('chat.flowProposal.save'));
    await waitFor(() => expect(mockSetFlowEnabled).toHaveBeenCalledWith('f1', true));
    expect(mockDispatch).toHaveBeenCalledTimes(1);
  });

  it('keeps the flow saved and lets the user retry just the enable step if setFlowEnabled fails', async () => {
    mockCreateFlow.mockResolvedValueOnce({
      id: 'f1',
      name: 'Daily standup summary',
      enabled: false,
    });
    mockSetFlowEnabled.mockRejectedValueOnce(new Error('enable boom'));
    render(<WorkflowProposalCard threadId="t1" proposal={proposal()} />);
    fireEvent.click(screen.getByText('chat.flowProposal.save'));
    await waitFor(() =>
      expect(screen.getByText(/chat\.flowProposal\.enableError/)).toBeInTheDocument()
    );
    // The flow was already persisted — the card must not be cleared, and a
    // retry must not re-create it (which would duplicate the flow).
    expect(mockDispatch).not.toHaveBeenCalled();

    mockSetFlowEnabled.mockResolvedValueOnce({ id: 'f1', enabled: true });
    fireEvent.click(screen.getByText('chat.flowProposal.save'));
    await waitFor(() => expect(mockDispatch).toHaveBeenCalledTimes(1));
    // Only ever created once, even though "Save & enable" was clicked twice.
    expect(mockCreateFlow).toHaveBeenCalledTimes(1);
    expect(mockSetFlowEnabled).toHaveBeenCalledTimes(2);
    expect(mockSetFlowEnabled).toHaveBeenLastCalledWith('f1', true);
  });

  it('opens the proposed graph in the canvas as an unsaved draft without persisting', () => {
    const p = proposal();
    render(<WorkflowProposalCard threadId="t1" proposal={p} />);
    fireEvent.click(screen.getByText('chat.flowProposal.openInCanvas'));

    // Navigates to the draft canvas route, carrying the graph in router state.
    expect(mockNavigate).toHaveBeenCalledTimes(1);
    const [route, opts] = mockNavigate.mock.calls[0];
    expect(route).toBe('/flows/draft');
    expect(opts.state).toEqual({
      name: p.name,
      graph: p.graph,
      requireApproval: p.requireApproval,
    });

    // The single persistence gate is untouched — no create/update, and the
    // proposal is left intact in the thread (not dismissed).
    expect(mockCreateFlow).not.toHaveBeenCalled();
    expect(mockUpdateFlow).not.toHaveBeenCalled();
    expect(mockDispatch).not.toHaveBeenCalled();
  });

  it('dismiss clears the proposal without calling createFlow', () => {
    render(<WorkflowProposalCard threadId="t1" proposal={proposal()} />);
    fireEvent.click(screen.getByText('chat.flowProposal.dismiss'));
    expect(mockCreateFlow).not.toHaveBeenCalled();
    expect(mockDispatch).toHaveBeenCalledTimes(1);
  });

  it('renders a fallback message when there are no non-trigger steps', () => {
    render(
      <WorkflowProposalCard
        threadId="t1"
        proposal={proposal({ summary: { trigger: 'manual', steps: [] } })}
      />
    );
    expect(screen.getByText('chat.flowProposal.noSteps')).toBeInTheDocument();
  });

  it('shows the require-approval hint only when requireApproval is true', () => {
    const { rerender } = render(
      <WorkflowProposalCard threadId="t1" proposal={proposal({ requireApproval: true })} />
    );
    expect(screen.getByText('chat.flowProposal.requireApprovalHint')).toBeInTheDocument();

    rerender(
      <WorkflowProposalCard threadId="t1" proposal={proposal({ requireApproval: false })} />
    );
    expect(screen.queryByText('chat.flowProposal.requireApprovalHint')).not.toBeInTheDocument();
  });
});
