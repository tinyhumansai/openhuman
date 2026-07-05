import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { WorkflowProposal } from '../../store/chatRuntimeSlice';
import WorkflowPromptBar from './WorkflowPromptBar';

// Echo i18n keys.
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

// Stub the proposal card so we only assert it renders with the right props.
vi.mock('../chat/WorkflowProposalCard', () => ({
  default: ({ threadId, proposal }: { threadId: string; proposal: WorkflowProposal }) => (
    <div data-testid="stub-proposal-card">
      {threadId}:{proposal.name}
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

describe('WorkflowPromptBar', () => {
  beforeEach(() => {
    hookState.threadId = null;
    hookState.sending = false;
    hookState.proposal = null;
    hookState.error = null;
    hookState.send = vi.fn().mockResolvedValue(undefined);
    hookState.clearProposal = vi.fn();
  });

  it('submits a builder turn with a delegation prompt containing the description', async () => {
    render(<WorkflowPromptBar />);
    fireEvent.change(screen.getByTestId('workflow-prompt-input'), {
      target: { value: 'digest my Slack every morning' },
    });
    fireEvent.click(screen.getByTestId('workflow-prompt-submit'));

    expect(hookState.send).toHaveBeenCalledTimes(1);
    const arg = hookState.send.mock.calls[0][0];
    expect(arg.displayText).toBe('digest my Slack every morning');
    expect(arg.prompt).toContain('digest my Slack every morning');
    expect(arg.prompt.toLowerCase()).toContain('workflow builder');
  });

  it('does not submit empty/whitespace input', () => {
    render(<WorkflowPromptBar />);
    fireEvent.change(screen.getByTestId('workflow-prompt-input'), { target: { value: '   ' } });
    fireEvent.click(screen.getByTestId('workflow-prompt-submit'));
    expect(hookState.send).not.toHaveBeenCalled();
  });

  it('renders the resulting proposal inline via WorkflowProposalCard', () => {
    hookState.threadId = 'builder-thread-1';
    hookState.proposal = {
      name: 'Morning digest',
      graph: { nodes: [], edges: [] },
      requireApproval: true,
      summary: { trigger: 'schedule', steps: [] },
    };
    render(<WorkflowPromptBar />);
    const card = screen.getByTestId('stub-proposal-card');
    expect(card).toHaveTextContent('builder-thread-1:Morning digest');
  });

  it('shows the offline hint when the hook reports offline', () => {
    hookState.error = 'offline';
    render(<WorkflowPromptBar />);
    expect(screen.getByTestId('workflow-prompt-error')).toHaveTextContent(
      'flows.promptBar.offline'
    );
  });
});
