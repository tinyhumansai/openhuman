import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { FlowSuggestion } from '../../services/api/flowsApi';
import type { WorkflowProposal } from '../../store/chatRuntimeSlice';
import SuggestedWorkflows from './SuggestedWorkflows';

// Echo i18n keys so assertions can target them directly.
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

// Stub the proposal card — we only assert it renders with the right props.
vi.mock('../chat/WorkflowProposalCard', () => ({
  default: ({ proposal, onSaved }: { proposal: WorkflowProposal; onSaved?: () => void }) => (
    <div data-testid="stub-proposal-card">
      {proposal.name}
      <button data-testid="stub-save" onClick={() => onSaved?.()}>
        save
      </button>
    </div>
  ),
}));

const hookState = vi.hoisted(() => ({
  threadId: null as string | null,
  sending: false,
  proposal: null as WorkflowProposal | null,
  error: null as string | null,
  send: vi.fn(),
  clearProposal: vi.fn(),
}));
vi.mock('../../hooks/useWorkflowBuilderChat', () => ({ useWorkflowBuilderChat: () => hookState }));

const api = vi.hoisted(() => ({
  discoverWorkflows: vi.fn(),
  listSuggestions: vi.fn(),
  dismissSuggestion: vi.fn(),
  markSuggestionBuilt: vi.fn(),
}));
vi.mock('../../services/api/flowsApi', () => ({
  discoverWorkflows: (...a: unknown[]) => api.discoverWorkflows(...a),
  listSuggestions: (...a: unknown[]) => api.listSuggestions(...a),
  dismissSuggestion: (...a: unknown[]) => api.dismissSuggestion(...a),
  markSuggestionBuilt: (...a: unknown[]) => api.markSuggestionBuilt(...a),
}));

function suggestion(overrides: Partial<FlowSuggestion> = {}): FlowSuggestion {
  return {
    id: 'sug_1',
    title: 'Auto-file receipts',
    one_liner: 'Add each Gmail receipt to your sheet.',
    rationale: 'You forward receipts weekly.',
    trigger_hint: 'app_event',
    steps_outline: ['Watch Gmail'],
    suggested_connections: ['composio:gmail:c1'],
    suggested_slugs: [],
    build_prompt: 'Build a workflow that files receipts.',
    confidence: 0.8,
    status: 'new',
    created_at: '2026-07-05T00:00:00Z',
    source_run_id: null,
    ...overrides,
  };
}

describe('SuggestedWorkflows', () => {
  beforeEach(() => {
    hookState.threadId = null;
    hookState.sending = false;
    hookState.proposal = null;
    hookState.send = vi.fn().mockResolvedValue(undefined);
    api.discoverWorkflows = vi.fn().mockResolvedValue([]);
    api.listSuggestions = vi.fn().mockResolvedValue([]);
    api.dismissSuggestion = vi.fn().mockResolvedValue(true);
    api.markSuggestionBuilt = vi.fn().mockResolvedValue(true);
  });

  it('shows the empty state when there are no suggestions', async () => {
    render(<SuggestedWorkflows />);
    await waitFor(() => expect(api.listSuggestions).toHaveBeenCalledWith('new'));
    expect(screen.getByTestId('flow-suggestions-empty')).toBeInTheDocument();
  });

  it('loads persisted suggestions on mount and renders a card', async () => {
    api.listSuggestions = vi.fn().mockResolvedValue([suggestion()]);
    render(<SuggestedWorkflows />);
    await waitFor(() => expect(screen.getByTestId('flow-suggestion-card')).toBeInTheDocument());
    expect(screen.getByText('Auto-file receipts')).toBeInTheDocument();
  });

  it('runs discovery on Discover click and renders the returned suggestions', async () => {
    api.discoverWorkflows = vi.fn().mockResolvedValue([suggestion({ title: 'Fresh idea' })]);
    render(<SuggestedWorkflows />);
    await waitFor(() => expect(api.listSuggestions).toHaveBeenCalled());

    fireEvent.click(screen.getByTestId('flow-suggestions-discover'));

    await waitFor(() => expect(screen.getByText('Fresh idea')).toBeInTheDocument());
    expect(api.discoverWorkflows).toHaveBeenCalledTimes(1);
  });

  it('dismisses a suggestion optimistically and calls the API', async () => {
    api.listSuggestions = vi.fn().mockResolvedValue([suggestion()]);
    render(<SuggestedWorkflows />);
    await waitFor(() => expect(screen.getByTestId('flow-suggestion-card')).toBeInTheDocument());

    fireEvent.click(screen.getByTestId('flow-suggestion-dismiss'));

    await waitFor(() =>
      expect(screen.queryByTestId('flow-suggestion-card')).not.toBeInTheDocument()
    );
    expect(api.dismissSuggestion).toHaveBeenCalledWith('sug_1');
  });

  it('sends a builder turn with the build_prompt on Build this', async () => {
    api.listSuggestions = vi.fn().mockResolvedValue([suggestion()]);
    render(<SuggestedWorkflows />);
    await waitFor(() => expect(screen.getByTestId('flow-suggestion-card')).toBeInTheDocument());

    fireEvent.click(screen.getByTestId('flow-suggestion-build'));

    await waitFor(() => expect(hookState.send).toHaveBeenCalledTimes(1));
    const arg = hookState.send.mock.calls[0][0];
    expect(arg.displayText).toBe('Auto-file receipts');
    // The build_prompt is now the instruction of a structured create request;
    // the server renders the agent brief from it.
    expect(arg.request.mode).toBe('create');
    expect(arg.request.instruction).toBe('Build a workflow that files receipts.');
  });

  it('marks the suggestion built when the inline proposal is saved', async () => {
    api.listSuggestions = vi.fn().mockResolvedValue([suggestion()]);
    // Simulate the builder having returned a proposal on a thread.
    hookState.threadId = 'thread-1';
    hookState.proposal = {
      name: 'Auto-file receipts',
      graph: { nodes: [], edges: [] },
      requireApproval: true,
      summary: { trigger: 'app_event', steps: [] },
    } as unknown as WorkflowProposal;

    render(<SuggestedWorkflows />);
    await waitFor(() => expect(screen.getByTestId('flow-suggestion-card')).toBeInTheDocument());

    // Start building so buildingId is set, then save via the stubbed card.
    fireEvent.click(screen.getByTestId('flow-suggestion-build'));
    await waitFor(() => expect(screen.getByTestId('stub-proposal-card')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('stub-save'));

    await waitFor(() => expect(api.markSuggestionBuilt).toHaveBeenCalledWith('sug_1'));
    // Card dropped from the active list.
    expect(screen.queryByTestId('flow-suggestion-card')).not.toBeInTheDocument();
  });
});
