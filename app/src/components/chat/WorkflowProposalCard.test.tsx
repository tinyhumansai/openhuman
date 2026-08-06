import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { markWorkflowProposalCompleted, type WorkflowProposal } from '../../store/chatRuntimeSlice';
import WorkflowProposalCard from './WorkflowProposalCard';

// Echo i18n keys so we can assert on the stable key string.
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

// `vi.mock` factories are hoisted above the module's top-level statements, so
// every handle a factory closes over must be declared via `vi.hoisted` rather
// than a plain top-level `const` — otherwise it'd be a TDZ reference at the
// time the (hoisted) factory runs. (These specific names happened to work
// without it, since Vitest's compiler special-cases `mock`-prefixed
// identifiers, but that's an incidental heuristic, not a guarantee.)
const {
  mockCreateFlow,
  mockUpdateFlow,
  mockSetFlowEnabled,
  mockGetApprovalManifest,
  mockPreauthorizeFlow,
  mockDispatch,
  mockNavigate,
} = vi.hoisted(() => ({
  mockCreateFlow: vi.fn(),
  mockUpdateFlow: vi.fn(),
  mockSetFlowEnabled: vi.fn(),
  mockGetApprovalManifest: vi.fn(),
  mockPreauthorizeFlow: vi.fn(),
  mockDispatch: vi.fn(),
  mockNavigate: vi.fn(),
}));
vi.mock('../../services/api/flowsApi', () => ({
  createFlow: (...args: unknown[]) => mockCreateFlow(...args),
  updateFlow: (...args: unknown[]) => mockUpdateFlow(...args),
  setFlowEnabled: (...args: unknown[]) => mockSetFlowEnabled(...args),
  getApprovalManifest: (...args: unknown[]) => mockGetApprovalManifest(...args),
}));
vi.mock('../../services/api/approvalApi', () => ({
  preauthorizeFlow: (...args: unknown[]) => mockPreauthorizeFlow(...args),
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
    // Default: nothing missing — the pre-authorization card stays out of the
    // way and every pre-existing save-path expectation holds unchanged.
    mockGetApprovalManifest
      .mockReset()
      .mockResolvedValue({ entries: [], missing: [], already_trusted: [], gate_installed: true });
    mockPreauthorizeFlow
      .mockReset()
      .mockResolvedValue({ flow_id: 'f1', granted: [], already_trusted: [], gate_installed: true });
    mockDispatch.mockReset();
    mockNavigate.mockReset();
  });

  it('renders the name, trigger, and steps with plain-language node-kind badges', () => {
    render(<WorkflowProposalCard threadId="t1" proposal={proposal()} />);
    expect(screen.getByText('Daily standup summary')).toBeInTheDocument();
    expect(screen.getByText('schedule: 0 9 * * *')).toBeInTheDocument();
    expect(screen.getByText('Summarize')).toBeInTheDocument();
    expect(screen.getByText('Post to Slack')).toBeInTheDocument();
    // Badges show the friendly i18n key (useT is mocked to echo the key),
    // never the raw snake_case wire `kind`.
    expect(screen.getByText('chat.flowProposal.stepKind.agent')).toBeInTheDocument();
    expect(screen.getByText('chat.flowProposal.stepKind.toolCall')).toBeInTheDocument();
    expect(screen.queryByText('agent')).not.toBeInTheDocument();
    expect(screen.queryByText('tool_call')).not.toBeInTheDocument();
    expect(screen.getAllByTestId('workflow-proposal-step-kind')).toHaveLength(2);
  });

  it('hides config_hint from the card even when present on the step', () => {
    render(<WorkflowProposalCard threadId="t1" proposal={proposal()} />);
    // `proposal()` sets a config_hint on the first step; the card must not
    // surface it (it can leak internal identifiers like tool slugs or URLs).
    expect(screen.queryByText("Summarize yesterday's messages")).not.toBeInTheDocument();
  });

  it('falls back to a humanized label for an unrecognized step kind', () => {
    render(
      <WorkflowProposalCard
        threadId="t1"
        proposal={proposal({
          summary: {
            trigger: 'manual',
            steps: [{ kind: 'future_thing', name: 'Do the new thing' }],
          },
        })}
      />
    );
    // Not in STEP_KIND_I18N_KEYS, so it must fall back to the pure
    // capitalize + underscores-to-spaces helper rather than the raw kind.
    expect(screen.getByText('Future thing')).toBeInTheDocument();
    expect(screen.queryByText('future_thing')).not.toBeInTheDocument();
  });

  // `step.kind` is arbitrary wire data. A plain bracket index would resolve
  // inherited Object members (e.g. `constructor` -> the Object function) and
  // hand a non-string to t(), breaking the badge render. The own-property
  // guard must send these through the humanized fallback instead. `constructor`
  // and `toString` have no underscores, so the humanized label is just the
  // capitalized kind.
  it.each(['constructor', 'toString'])(
    'humanizes the inherited-property kind %s instead of resolving it on the prototype',
    kind => {
      render(
        <WorkflowProposalCard
          threadId="t1"
          proposal={proposal({
            summary: { trigger: 'manual', steps: [{ kind, name: 'Edge-case step' }] },
          })}
        />
      );
      const expected = kind.charAt(0).toUpperCase() + kind.slice(1);
      expect(screen.getByText(expected)).toBeInTheDocument();
      expect(screen.getByText('Edge-case step')).toBeInTheDocument();
    }
  );

  it('renders a __proto__ step kind without leaking an inherited property', () => {
    render(
      <WorkflowProposalCard
        threadId="t1"
        proposal={proposal({
          summary: { trigger: 'manual', steps: [{ kind: '__proto__', name: 'Proto step' }] },
        })}
      />
    );
    // The own-property guard treats __proto__ as unknown and humanizes it, so
    // the row still renders (step name present) and the badge shows a real
    // string. Assert the badge's exact (whitespace-normalized) content directly
    // — not just the step name / absence of "function" — so a missing badge or
    // an inherited-member leak like `[object Object]` would fail here too.
    // `__proto__` humanizes (underscores -> spaces, first char already a space)
    // to a lowercase "proto" badge.
    expect(screen.getByText('Proto step')).toBeInTheDocument();
    expect(screen.getByTestId('workflow-proposal-step-kind')).toHaveTextContent(/^proto$/);
  });

  it('has the expected root test id', () => {
    render(<WorkflowProposalCard threadId="t1" proposal={proposal()} />);
    expect(screen.getByTestId('workflow-proposal-card')).toBeInTheDocument();
  });

  it('saves via createFlow with the right args and shows the saved confirmation', async () => {
    const p = proposal();
    render(<WorkflowProposalCard threadId="t1" proposal={p} />);
    fireEvent.click(screen.getByText('chat.flowProposal.save'));
    await waitFor(() =>
      expect(mockCreateFlow).toHaveBeenCalledWith(p.name, p.graph, p.requireApproval)
    );
    // createFlow already came back enabled — no need for a follow-up arm.
    expect(mockSetFlowEnabled).not.toHaveBeenCalled();
    // The card stays mounted (issue B36) showing a saved confirmation with a
    // link into the persisted flow — it must not silently clear/unmount, so
    // the proposal is NOT dispatched away until the user follows that link.
    await waitFor(() => expect(screen.getByTestId('workflow-proposal-saved')).toBeInTheDocument());
    expect(screen.getByText('chat.flowProposal.savedConfirmation')).toBeInTheDocument();
    // The completion IS mirrored into Redux (not just local state) so the
    // confirmation survives a remount before "View workflow" is clicked —
    // see `WorkflowProposal.completedFlowId`. It must not be the
    // proposal-clearing dispatch, though — that only happens on "View
    // workflow"/"Dismiss".
    expect(mockDispatch).toHaveBeenCalledTimes(1);
    expect(mockDispatch).toHaveBeenCalledWith(
      markWorkflowProposalCompleted({ threadId: 't1', flowId: 'f1' })
    );
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it('navigates to the persisted flow and clears the proposal when "View workflow" is clicked', async () => {
    const p = proposal();
    render(<WorkflowProposalCard threadId="t1" proposal={p} />);
    fireEvent.click(screen.getByText('chat.flowProposal.save'));
    await waitFor(() =>
      expect(screen.getByText('chat.flowProposal.viewWorkflow')).toBeInTheDocument()
    );
    // The successful save already dispatched markWorkflowProposalCompleted;
    // record how many calls happened before "View workflow" is clicked so
    // the assertions below are about what that click itself does.
    const callsBeforeClick = mockDispatch.mock.calls.length;

    fireEvent.click(screen.getByText('chat.flowProposal.viewWorkflow'));

    // Navigates straight to the saved flow's own canvas route — the created
    // flow's real id ('f1'), not the unsaved-draft route.
    expect(mockNavigate).toHaveBeenCalledWith('/flows/f1');
    expect(mockDispatch).toHaveBeenCalledTimes(callsBeforeClick + 1);
    // Assert the RELATIVE order, not just that both happened: the
    // proposal-clearing dispatch must fire before navigation away, since the
    // parent only renders this card while the proposal survives in Redux —
    // navigating first (even a tick early) risks unmounting before the
    // dispatch lands.
    const dispatchOrder = mockDispatch.mock.invocationCallOrder[callsBeforeClick];
    const navigateOrder = mockNavigate.mock.invocationCallOrder[0];
    expect(dispatchOrder).toBeLessThan(navigateOrder);
  });

  // Regression test for the remount bug flagged in review (issue B36): the
  // card intentionally stays mounted showing the saved confirmation instead
  // of dispatching the proposal away, so a thread switch / route change can
  // unmount and remount it before the user clicks "View workflow". Before
  // `completedFlowId` was mirrored into Redux, remounting reset the card's
  // local state to null, so it fell back to the pre-save editable view — and
  // a second "Save & enable" click would call `createFlow` again and
  // duplicate the flow.
  it('keeps showing the saved confirmation across a remount instead of re-offering "Save & enable"', async () => {
    const p = proposal();
    const { unmount } = render(<WorkflowProposalCard threadId="t1" proposal={p} />);
    fireEvent.click(screen.getByText('chat.flowProposal.save'));
    await waitFor(() => expect(screen.getByTestId('workflow-proposal-saved')).toBeInTheDocument());

    // Simulate a thread switch / route change: unmount the card, then
    // remount it with the proposal as it would now read from Redux — still
    // present (the completion dispatch didn't clear it), but carrying the
    // `completedFlowId` set by `markWorkflowProposalCompleted`.
    unmount();
    mockCreateFlow.mockClear();
    render(<WorkflowProposalCard threadId="t1" proposal={{ ...p, completedFlowId: 'f1' }} />);

    // Must still show the saved confirmation, not the editable "Save &
    // enable" view that would let a second click duplicate the flow.
    expect(screen.getByTestId('workflow-proposal-saved')).toBeInTheDocument();
    expect(screen.queryByText('chat.flowProposal.save')).not.toBeInTheDocument();
    expect(mockCreateFlow).not.toHaveBeenCalled();
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
    // The proposal isn't dispatched away immediately — the card shows the
    // saved confirmation with a view link instead (issue B36). The only
    // dispatch is the Redux-mirrored completion (survives a remount), not a
    // clear.
    await waitFor(() => expect(screen.getByTestId('workflow-proposal-saved')).toBeInTheDocument());
    expect(mockDispatch).toHaveBeenCalledTimes(1);
    expect(mockDispatch).toHaveBeenCalledWith(
      markWorkflowProposalCompleted({ threadId: 't1', flowId: 'f1' })
    );
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
    await waitFor(() => expect(screen.getByTestId('workflow-proposal-saved')).toBeInTheDocument());
    // Only ever created once, even though "Save & enable" was clicked twice.
    expect(mockCreateFlow).toHaveBeenCalledTimes(1);
    expect(mockSetFlowEnabled).toHaveBeenCalledTimes(2);
    expect(mockSetFlowEnabled).toHaveBeenLastCalledWith('f1', true);
    // Still not dispatched away — the retry lands on the same saved
    // confirmation, not an immediate clear. The only dispatch is the
    // Redux-mirrored completion once the retry succeeds.
    expect(mockDispatch).toHaveBeenCalledTimes(1);
    expect(mockDispatch).toHaveBeenCalledWith(
      markWorkflowProposalCompleted({ threadId: 't1', flowId: 'f1' })
    );
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

describe('WorkflowProposalCard pre-authorization', () => {
  const MISSING_MANIFEST = {
    entries: [
      {
        kind: 'approvable',
        node_id: 'n1',
        tool_name: 'GMAIL_SEND_EMAIL',
        label: 'Use GMAIL_SEND_EMAIL',
      },
    ],
    missing: ['GMAIL_SEND_EMAIL'],
    already_trusted: [],
    gate_installed: true,
  };

  it('surfaces the consolidated card inline when the saved flow has missing grants', async () => {
    mockGetApprovalManifest.mockResolvedValue(MISSING_MANIFEST);
    render(<WorkflowProposalCard threadId="t1" proposal={proposal()} />);

    fireEvent.click(screen.getByText('chat.flowProposal.save'));

    await waitFor(() =>
      expect(screen.getByTestId('flow-preauthorization-card')).toBeInTheDocument()
    );
    // Not completed yet — the decision is still pending.
    expect(mockDispatch).not.toHaveBeenCalled();
    expect(screen.queryByTestId('workflow-proposal-saved')).not.toBeInTheDocument();
  });

  it('Approve all grants the tools and lands in the terminal saved view', async () => {
    mockGetApprovalManifest.mockResolvedValue(MISSING_MANIFEST);
    render(<WorkflowProposalCard threadId="t1" proposal={proposal()} />);

    fireEvent.click(screen.getByText('chat.flowProposal.save'));
    await waitFor(() =>
      expect(screen.getByTestId('flow-preauthorization-card')).toBeInTheDocument()
    );
    fireEvent.click(screen.getByText('flows.enableApproval.approveAll'));

    await waitFor(() => expect(screen.getByTestId('workflow-proposal-saved')).toBeInTheDocument());
    expect(mockPreauthorizeFlow).toHaveBeenCalledWith('f1', ['GMAIL_SEND_EMAIL']);
    expect(mockDispatch).toHaveBeenCalledWith(
      markWorkflowProposalCompleted({ threadId: 't1', flowId: 'f1' })
    );
    // checkAfterSave path: the flow came back enabled from createFlow, so no
    // redundant enable call fires on approve.
    expect(mockSetFlowEnabled).not.toHaveBeenCalled();
  });

  it('re-clicking Save while the card is open never creates a duplicate flow', async () => {
    mockGetApprovalManifest.mockResolvedValue(MISSING_MANIFEST);
    render(<WorkflowProposalCard threadId="t1" proposal={proposal()} />);

    fireEvent.click(screen.getByText('chat.flowProposal.save'));
    await waitFor(() =>
      expect(screen.getByTestId('flow-preauthorization-card')).toBeInTheDocument()
    );
    // The proposal's own Save button is clickable again (saving released
    // while the decision is pending) — a second click must reuse the
    // persisted flow id, not call createFlow again (greptile P1).
    fireEvent.click(screen.getByText('chat.flowProposal.save'));
    await waitFor(() => expect(mockGetApprovalManifest).toHaveBeenCalledTimes(2));
    expect(mockCreateFlow).toHaveBeenCalledTimes(1);
  });

  it('Deny leaves the flow saved but disabled with the informational note', async () => {
    mockGetApprovalManifest.mockResolvedValue(MISSING_MANIFEST);
    render(<WorkflowProposalCard threadId="t1" proposal={proposal()} />);

    fireEvent.click(screen.getByText('chat.flowProposal.save'));
    await waitFor(() =>
      expect(screen.getByTestId('flow-preauthorization-card')).toBeInTheDocument()
    );
    fireEvent.click(screen.getByText('flows.enableApproval.deny'));

    await waitFor(() => expect(mockSetFlowEnabled).toHaveBeenCalledWith('f1', false));
    await waitFor(() =>
      expect(screen.getByText(/flows.enableApproval.deniedDisabled/)).toBeInTheDocument()
    );
    // No terminal completion — the proposal card stays for a later retry.
    expect(mockDispatch).not.toHaveBeenCalledWith(
      markWorkflowProposalCompleted({ threadId: 't1', flowId: 'f1' })
    );
    expect(screen.queryByTestId('workflow-proposal-saved')).not.toBeInTheDocument();
  });
});
