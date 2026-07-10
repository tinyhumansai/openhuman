import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { clearConfigChat } from './ConfigAssistantPanel';
import ConfigHelpModal from './ConfigHelpModal';

const mockConfigAssist = vi.fn();

vi.mock('../../../services/api/mcpClientsApi', () => ({
  mcpClientsApi: { configAssist: (...args: unknown[]) => mockConfigAssist(...args) },
}));

describe('ConfigHelpModal', () => {
  beforeEach(() => {
    mockConfigAssist.mockReset();
    // The embedded ConfigAssistantPanel caches chat history per qualified_name at
    // module scope; clear it so each test starts with a fresh chat (and the
    // one-click setup-help action is offered again).
    clearConfigChat('acme/test-server');
    // The on-demand prompt resolves when the user triggers it.
    mockConfigAssist.mockResolvedValue({ reply: 'Get a token from the dashboard.' });
  });

  it('renders the modal with the help heading and embedded assistant panel', async () => {
    render(
      <ConfigHelpModal
        qualifiedName="acme/test-server"
        displayName="Test Server"
        description="A test MCP server"
        onClose={() => {}}
      />
    );
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    // Heading uses the "How do I get a token?" label.
    expect(screen.getByRole('heading', { name: 'Help & configure' })).toBeInTheDocument();
    // Embedded ConfigAssistantPanel renders its input.
    expect(screen.getByPlaceholderText(/ask a question/i)).toBeInTheDocument();
    // The help opens instantly — no blocking LLM call on open (#4272). The
    // research is offered as a one-click action instead.
    expect(mockConfigAssist).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Get step-by-step setup help' })).toBeInTheDocument();
  });

  it('runs a server-specific prompt on demand, naming the display name and qualified name', async () => {
    render(
      <ConfigHelpModal
        qualifiedName="acme/test-server"
        displayName="Test Server"
        description="A test MCP server"
        onClose={() => {}}
      />
    );
    // Nothing fires until the user asks for help.
    expect(mockConfigAssist).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Get step-by-step setup help' }));
    await waitFor(() => {
      expect(mockConfigAssist).toHaveBeenCalledTimes(1);
    });
    const [{ qualified_name, user_message }] = mockConfigAssist.mock.calls[0];
    expect(qualified_name).toBe('acme/test-server');
    // The prompt embeds the friendly name, the qualified name, and description.
    expect(user_message).toContain('Test Server');
    expect(user_message).toContain('acme/test-server');
    expect(user_message).toContain('A test MCP server');
  });

  it('builds an on-demand prompt that omits the description sentence when none is given', async () => {
    render(
      <ConfigHelpModal
        qualifiedName="acme/test-server"
        displayName="Test Server"
        onClose={() => {}}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: 'Get step-by-step setup help' }));
    await waitFor(() => {
      expect(mockConfigAssist).toHaveBeenCalledTimes(1);
    });
    const [{ user_message }] = mockConfigAssist.mock.calls[0];
    expect(user_message).toContain('Test Server');
    expect(user_message).toContain('acme/test-server');
  });

  it('closes via the ✕ button', () => {
    const onClose = vi.fn();
    render(
      <ConfigHelpModal
        qualifiedName="acme/test-server"
        displayName="Test Server"
        onClose={onClose}
      />
    );
    // The ✕ close button is labelled with the Cancel a11y string.
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onClose).toHaveBeenCalled();
  });

  it('closes on backdrop mousedown', () => {
    const onClose = vi.fn();
    render(
      <ConfigHelpModal
        qualifiedName="acme/test-server"
        displayName="Test Server"
        onClose={onClose}
      />
    );
    const dialog = screen.getByRole('dialog');
    fireEvent.mouseDown(dialog);
    expect(onClose).toHaveBeenCalled();
  });

  it('forwards onApplySuggestedEnv through to the assistant panel', async () => {
    mockConfigAssist.mockResolvedValue({
      reply: 'Here are values',
      suggested_env: { API_KEY: 'abc' },
    });
    const onApply = vi.fn();
    render(
      <ConfigHelpModal
        qualifiedName="acme/test-server"
        displayName="Test Server"
        onClose={() => {}}
        onApplySuggestedEnv={onApply}
      />
    );
    // Trigger the on-demand help; the reply carries suggested_env, so the Apply
    // button appears.
    fireEvent.click(screen.getByRole('button', { name: 'Get step-by-step setup help' }));
    const applyBtn = await screen.findByRole('button', { name: 'Apply suggested values' });
    fireEvent.click(applyBtn);
    expect(onApply).toHaveBeenCalledWith({ API_KEY: 'abc' });
  });
});
