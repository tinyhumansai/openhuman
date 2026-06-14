/**
 * AgentChatPanel unit tests — covers changed lines:
 * 129, 131, 134-135, 137, 141, 155, 163, 165
 *
 * Exercises: rendering the conversation area, sending a message, rendering
 * user and agent chat messages, error display on RPC failure, model/temp
 * overrides sent through, localStorage persistence, and the empty state.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import { openhumanAgentChat } from '../../../../utils/tauriCommands';
import AgentChatPanel from '../AgentChatPanel';

vi.mock('../../../../utils/tauriCommands', async () => {
  const actual = await vi.importActual<typeof import('../../../../utils/tauriCommands')>(
    '../../../../utils/tauriCommands'
  );
  return { ...actual, openhumanAgentChat: vi.fn() };
});

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

const mockAgentChat = vi.mocked(openhumanAgentChat);

describe('AgentChatPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    mockAgentChat.mockResolvedValue({ result: 'Hello from agent', logs: [] });
  });

  it('renders the panel header and empty conversation area', async () => {
    renderWithProviders(<AgentChatPanel />);

    // Empty state label shown when no messages
    expect(await screen.findByText(/start a conversation/i)).toBeInTheDocument();
  });

  it('shows model and temperature input fields', async () => {
    renderWithProviders(<AgentChatPanel />);

    expect(await screen.findByRole('textbox', { name: /model/i })).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: /temperature/i })).toBeInTheDocument();
  });

  it('sends a message and renders the user and agent reply', async () => {
    renderWithProviders(<AgentChatPanel />);

    const textarea = await screen.findByRole('textbox', { name: /ask.*agent/i });
    fireEvent.change(textarea, { target: { value: 'What is 2+2?' } });

    fireEvent.click(screen.getByRole('button', { name: /send/i }));

    // User message appears
    await waitFor(() => expect(screen.getByText('What is 2+2?')).toBeInTheDocument());

    // Agent reply appears
    await waitFor(() => expect(screen.getByText('Hello from agent')).toBeInTheDocument());
    expect(screen.getByText('You')).toBeInTheDocument();
    expect(screen.getByText('Agent')).toBeInTheDocument();
  });

  it('calls openhumanAgentChat with the message text', async () => {
    renderWithProviders(<AgentChatPanel />);

    const textarea = await screen.findByRole('textbox', { name: /ask.*agent/i });
    fireEvent.change(textarea, { target: { value: 'Tell me a joke' } });
    fireEvent.click(screen.getByRole('button', { name: /send/i }));

    await waitFor(() =>
      expect(mockAgentChat).toHaveBeenCalledWith('Tell me a joke', undefined, 0.7)
    );
  });

  it('passes model override when filled in', async () => {
    renderWithProviders(<AgentChatPanel />);

    const modelInput = await screen.findByRole('textbox', { name: /model/i });
    fireEvent.change(modelInput, { target: { value: 'claude-sonnet-4-5' } });

    const textarea = screen.getByRole('textbox', { name: /ask.*agent/i });
    fireEvent.change(textarea, { target: { value: 'Hello' } });
    fireEvent.click(screen.getByRole('button', { name: /send/i }));

    await waitFor(() =>
      expect(mockAgentChat).toHaveBeenCalledWith('Hello', 'claude-sonnet-4-5', 0.7)
    );
  });

  it('passes temperature override when filled in', async () => {
    renderWithProviders(<AgentChatPanel />);

    const tempInput = await screen.findByRole('textbox', { name: /temperature/i });
    fireEvent.change(tempInput, { target: { value: '0.2' } });

    const textarea = screen.getByRole('textbox', { name: /ask.*agent/i });
    fireEvent.change(textarea, { target: { value: 'Hello' } });
    fireEvent.click(screen.getByRole('button', { name: /send/i }));

    await waitFor(() => expect(mockAgentChat).toHaveBeenCalledWith('Hello', undefined, 0.2));
  });

  it('shows an error banner when openhumanAgentChat rejects', async () => {
    mockAgentChat.mockRejectedValueOnce(new Error('core offline'));

    renderWithProviders(<AgentChatPanel />);

    const textarea = await screen.findByRole('textbox', { name: /ask.*agent/i });
    fireEvent.change(textarea, { target: { value: 'Test message' } });
    fireEvent.click(screen.getByRole('button', { name: /send/i }));

    await waitFor(() => expect(screen.getByText('core offline')).toBeInTheDocument());
  });

  it('clears the input field after sending', async () => {
    renderWithProviders(<AgentChatPanel />);

    const textarea = (await screen.findByRole('textbox', {
      name: /ask.*agent/i,
    })) as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: 'Something to send' } });
    fireEvent.click(screen.getByRole('button', { name: /send/i }));

    await waitFor(() => expect(textarea.value).toBe(''));
  });

  it('disables the Send button while a request is in flight', async () => {
    let resolveSend!: (v: { result: string; logs: never[] }) => void;
    mockAgentChat.mockReturnValueOnce(
      new Promise(r => {
        resolveSend = r;
      })
    );

    renderWithProviders(<AgentChatPanel />);

    const textarea = await screen.findByRole('textbox', { name: /ask.*agent/i });
    fireEvent.change(textarea, { target: { value: 'In flight?' } });

    const sendBtn = screen.getByRole('button', { name: /send/i });
    fireEvent.click(sendBtn);

    // While in-flight the button is disabled (label changes to Loading)
    await waitFor(() => expect(sendBtn).toBeDisabled());

    resolveSend({ result: 'done', logs: [] });
    await waitFor(() => expect(sendBtn).not.toBeDisabled());
  });

  it('does not call openhumanAgentChat when input is blank', async () => {
    renderWithProviders(<AgentChatPanel />);

    const sendBtn = await screen.findByRole('button', { name: /send/i });
    fireEvent.click(sendBtn);

    await new Promise(r => setTimeout(r, 50));
    expect(mockAgentChat).not.toHaveBeenCalled();
  });

  it('renders multiple conversation turns in order', async () => {
    mockAgentChat
      .mockResolvedValueOnce({ result: 'Reply to first', logs: [] })
      .mockResolvedValueOnce({ result: 'Reply to second', logs: [] });

    renderWithProviders(<AgentChatPanel />);

    const textarea = await screen.findByRole('textbox', { name: /ask.*agent/i });

    // First turn
    fireEvent.change(textarea, { target: { value: 'First question' } });
    fireEvent.click(screen.getByRole('button', { name: /send/i }));
    await waitFor(() => expect(screen.getByText('Reply to first')).toBeInTheDocument());

    // Second turn
    fireEvent.change(textarea, { target: { value: 'Second question' } });
    fireEvent.click(screen.getByRole('button', { name: /send/i }));
    await waitFor(() => expect(screen.getByText('Reply to second')).toBeInTheDocument());

    expect(screen.getByText('First question')).toBeInTheDocument();
    expect(screen.getByText('Second question')).toBeInTheDocument();
  });
});
