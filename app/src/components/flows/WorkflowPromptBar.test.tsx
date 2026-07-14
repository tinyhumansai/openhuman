import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import WorkflowPromptBar from './WorkflowPromptBar';

// Echo i18n keys.
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

const navigateMock = vi.hoisted(() => vi.fn());
vi.mock('react-router-dom', () => ({ useNavigate: () => navigateMock }));

const createFlowMock = vi.hoisted(() => vi.fn());
vi.mock('../../services/api/flowsApi', () => ({
  createFlow: (...args: unknown[]) => createFlowMock(...args),
}));

describe('WorkflowPromptBar', () => {
  beforeEach(() => {
    navigateMock.mockReset();
    createFlowMock.mockReset();
    createFlowMock.mockResolvedValue({ id: 'flow-1', name: 'digest my Slack every morning' });
  });

  it('creates a flow named from the prompt and opens its canvas with a build seed', async () => {
    render(<WorkflowPromptBar />);
    fireEvent.change(screen.getByTestId('workflow-prompt-input'), {
      target: { value: 'digest my Slack every morning' },
    });
    fireEvent.click(screen.getByTestId('workflow-prompt-submit'));

    await waitFor(() => expect(navigateMock).toHaveBeenCalledTimes(1));
    expect(createFlowMock).toHaveBeenCalledTimes(1);
    const [name, graph, requireApproval] = createFlowMock.mock.calls[0];
    expect(name).toBe('digest my Slack every morning');
    // The created flow is the standard blank graph (single manual trigger).
    expect(graph.nodes).toHaveLength(1);
    expect(graph.nodes[0].kind).toBe('trigger');
    // Parity with the proposal-card path: prompt-authored flows must default
    // to requiring approval, matching WorkflowProposalCard's default.
    expect(requireApproval).toBe(true);
    expect(navigateMock).toHaveBeenCalledWith('/flows/flow-1', {
      state: { copilotBuild: { description: 'digest my Slack every morning' } },
    });
  });

  it('defaults requireApproval to true so outbound side-effects need explicit approval', async () => {
    render(<WorkflowPromptBar />);
    fireEvent.change(screen.getByTestId('workflow-prompt-input'), {
      target: { value: 'auto-reply to every gmail thread' },
    });
    fireEvent.click(screen.getByTestId('workflow-prompt-submit'));

    await waitFor(() => expect(createFlowMock).toHaveBeenCalledTimes(1));
    // Third positional arg must be `true` — flows_create's server default is
    // `false`, so omitting it would silently disarm the approval gate.
    expect(createFlowMock.mock.calls[0][2]).toBe(true);
  });

  it('submits on Enter (Shift+Enter reserved for newlines)', async () => {
    render(<WorkflowPromptBar />);
    const input = screen.getByTestId('workflow-prompt-input');
    fireEvent.change(input, { target: { value: 'ping me daily' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(createFlowMock).toHaveBeenCalledTimes(1));
  });

  it('does not submit empty/whitespace input', () => {
    render(<WorkflowPromptBar />);
    fireEvent.change(screen.getByTestId('workflow-prompt-input'), { target: { value: '   ' } });
    fireEvent.click(screen.getByTestId('workflow-prompt-submit'));
    expect(createFlowMock).not.toHaveBeenCalled();
    expect(navigateMock).not.toHaveBeenCalled();
  });

  it('shows an error and re-enables the composer when create fails', async () => {
    createFlowMock.mockRejectedValue(new Error('boom'));
    render(<WorkflowPromptBar />);
    fireEvent.change(screen.getByTestId('workflow-prompt-input'), {
      target: { value: 'digest my Slack every morning' },
    });
    fireEvent.click(screen.getByTestId('workflow-prompt-submit'));

    const error = await screen.findByTestId('workflow-prompt-error');
    expect(error).toHaveTextContent('flows.promptBar.error');
    expect(navigateMock).not.toHaveBeenCalled();
    expect(screen.getByTestId('workflow-prompt-input')).not.toBeDisabled();
  });
});
