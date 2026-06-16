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
    // module scope; clear it so each test starts with a fresh chat and the
    // auto-prompt fires again (a restored, non-empty chat suppresses the auto-run).
    clearConfigChat('acme/test-server');
    // The auto-sent prompt resolves so the auto-run doesn't surface as an error.
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
  });

  it('auto-runs a server-specific prompt naming the display name and qualified name', async () => {
    render(
      <ConfigHelpModal
        qualifiedName="acme/test-server"
        displayName="Test Server"
        description="A test MCP server"
        onClose={() => {}}
      />
    );
    await waitFor(() => {
      expect(mockConfigAssist).toHaveBeenCalledTimes(1);
    });
    const [{ qualified_name, user_message }] = mockConfigAssist.mock.calls[0];
    expect(qualified_name).toBe('acme/test-server');
    // The auto prompt embeds both the friendly name and the qualified name and
    // the description.
    expect(user_message).toContain('Test Server');
    expect(user_message).toContain('acme/test-server');
    expect(user_message).toContain('A test MCP server');
  });

  it('builds an auto prompt that omits the description sentence when none is given', async () => {
    render(
      <ConfigHelpModal
        qualifiedName="acme/test-server"
        displayName="Test Server"
        onClose={() => {}}
      />
    );
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
    // The auto-run reply carries suggested_env, so the Apply button appears.
    const applyBtn = await screen.findByRole('button', { name: 'Apply suggested values' });
    fireEvent.click(applyBtn);
    expect(onApply).toHaveBeenCalledWith({ API_KEY: 'abc' });
  });
});
