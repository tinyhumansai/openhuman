import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import ConfigAssistantPanel from './ConfigAssistantPanel';

const mockConfigAssist = vi.fn();

vi.mock('../../../services/api/mcpClientsApi', () => ({
  mcpClientsApi: { configAssist: (...args: unknown[]) => mockConfigAssist(...args) },
}));

describe('ConfigAssistantPanel', () => {
  beforeEach(() => {
    mockConfigAssist.mockReset();
  });

  it('renders the input textarea and Send button', () => {
    render(<ConfigAssistantPanel qualifiedName="acme/test" />);
    expect(screen.getByPlaceholderText(/ask a question/i)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Send' })).toBeInTheDocument();
  });

  it('send button is disabled when input is empty', () => {
    render(<ConfigAssistantPanel qualifiedName="acme/test" />);
    expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
  });

  it('enables send button when input has text', () => {
    render(<ConfigAssistantPanel qualifiedName="acme/test" />);
    fireEvent.change(screen.getByPlaceholderText(/ask a question/i), {
      target: { value: 'What env vars do I need?' },
    });
    expect(screen.getByRole('button', { name: 'Send' })).not.toBeDisabled();
  });

  it('sends message and renders assistant reply', async () => {
    mockConfigAssist.mockResolvedValue({ reply: 'You need an API_KEY env var.' });
    render(<ConfigAssistantPanel qualifiedName="acme/test" />);

    fireEvent.change(screen.getByPlaceholderText(/ask a question/i), {
      target: { value: 'What do I need?' },
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    });

    await waitFor(() => {
      expect(screen.getByText('You need an API_KEY env var.')).toBeInTheDocument();
    });

    expect(mockConfigAssist).toHaveBeenCalledWith({
      qualified_name: 'acme/test',
      user_message: 'What do I need?',
      history: [{ role: 'user', content: 'What do I need?' }],
    });
  });

  it('shows suggested_env values and Apply button', async () => {
    mockConfigAssist.mockResolvedValue({
      reply: 'Here are suggested values',
      suggested_env: { API_KEY: 'abc123' },
    });

    const onApply = vi.fn();
    render(<ConfigAssistantPanel qualifiedName="acme/test" onApplySuggestedEnv={onApply} />);

    fireEvent.change(screen.getByPlaceholderText(/ask a question/i), {
      target: { value: 'Help me configure' },
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    });

    await waitFor(() => {
      expect(screen.getByText('Here are suggested values')).toBeInTheDocument();
    });

    // Shows key name (the colon is in the same text node with whitespace)
    expect(screen.getByText(/API_KEY:/)).toBeInTheDocument();

    // Apply button exists and calls the callback
    const applyBtn = screen.getByRole('button', { name: 'Apply suggested values' });
    fireEvent.click(applyBtn);
    expect(onApply).toHaveBeenCalledWith({ API_KEY: 'abc123' });
  });

  it('shows error on failed request', async () => {
    mockConfigAssist.mockRejectedValue(new Error('AI service unavailable'));
    render(<ConfigAssistantPanel qualifiedName="acme/test" />);

    fireEvent.change(screen.getByPlaceholderText(/ask a question/i), {
      target: { value: 'Hello' },
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    });

    await waitFor(() => {
      expect(screen.getByText('AI service unavailable')).toBeInTheDocument();
    });
  });

  it('clears input after sending', async () => {
    mockConfigAssist.mockResolvedValue({ reply: 'OK' });
    render(<ConfigAssistantPanel qualifiedName="acme/test" />);

    const textarea = screen.getByPlaceholderText(/ask a question/i) as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: 'Question?' } });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Send' }));
    });

    expect(textarea.value).toBe('');
  });

  it('sends on Enter key press', async () => {
    mockConfigAssist.mockResolvedValue({ reply: 'reply' });
    render(<ConfigAssistantPanel qualifiedName="acme/test" />);

    const textarea = screen.getByPlaceholderText(/ask a question/i);
    fireEvent.change(textarea, { target: { value: 'test message' } });

    await act(async () => {
      fireEvent.keyDown(textarea, { key: 'Enter', shiftKey: false });
    });

    await waitFor(() => {
      expect(mockConfigAssist).toHaveBeenCalledTimes(1);
    });
  });
});
