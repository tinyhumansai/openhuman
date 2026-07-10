import { fireEvent, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { FALLBACK_DEFINITIONS } from '../../../lib/channels/definitions';
import { setChannelConnectionStatus } from '../../../store/channelConnectionsSlice';
import { createTestStore, renderWithProviders } from '../../../test/test-utils';
import ChannelSelector from '../ChannelSelector';

describe('ChannelSelector', () => {
  const onSelect = vi.fn();

  it('renders all channel tabs', () => {
    renderWithProviders(
      <ChannelSelector
        definitions={FALLBACK_DEFINITIONS}
        selectedChannel="telegram"
        onSelectChannel={onSelect}
      />
    );

    expect(screen.getByText('Telegram')).toBeInTheDocument();
    expect(screen.getByText('Discord')).toBeInTheDocument();
    expect(screen.getByText('Web')).toBeInTheDocument();
  });

  it('calls onSelectChannel when a tab is clicked', () => {
    renderWithProviders(
      <ChannelSelector
        definitions={FALLBACK_DEFINITIONS}
        selectedChannel="telegram"
        onSelectChannel={onSelect}
      />
    );

    fireEvent.click(screen.getByText('Discord'));
    expect(onSelect).toHaveBeenCalledWith('discord');
  });

  it('shows active route summary', () => {
    renderWithProviders(
      <ChannelSelector
        definitions={FALLBACK_DEFINITIONS}
        selectedChannel="telegram"
        onSelectChannel={onSelect}
      />
    );

    expect(screen.getByText(/No active route/)).toBeInTheDocument();
  });

  it('surfaces channel errors when no mode is connected or connecting', () => {
    const store = createTestStore();
    store.dispatch(
      setChannelConnectionStatus({
        channel: 'telegram',
        authMode: 'bot_token',
        status: 'error',
        lastError: 'Invalid token',
      })
    );

    renderWithProviders(
      <ChannelSelector
        definitions={FALLBACK_DEFINITIONS}
        selectedChannel="telegram"
        onSelectChannel={onSelect}
      />,
      { store }
    );

    const telegramTab = screen.getByRole('button', { name: /telegram/i });
    expect(within(telegramTab).getByText('Error')).toBeInTheDocument();
    expect(within(telegramTab).queryByText('Disconnected')).not.toBeInTheDocument();
  });

  it('renders the MCP virtual tab', () => {
    renderWithProviders(
      <ChannelSelector
        definitions={FALLBACK_DEFINITIONS}
        selectedChannel="telegram"
        onSelectChannel={onSelect}
      />
    );
    expect(screen.getByRole('button', { name: /mcp servers/i })).toBeInTheDocument();
  });

  it('calls onSelectChannel with "mcp" when MCP tab is clicked', () => {
    const handleSelect = vi.fn();
    renderWithProviders(
      <ChannelSelector
        definitions={FALLBACK_DEFINITIONS}
        selectedChannel="telegram"
        onSelectChannel={handleSelect}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /mcp servers/i }));
    expect(handleSelect).toHaveBeenCalledWith('mcp');
  });

  it('applies selected styling to MCP tab when it is the active channel', () => {
    renderWithProviders(
      <ChannelSelector
        definitions={FALLBACK_DEFINITIONS}
        selectedChannel="mcp"
        onSelectChannel={onSelect}
      />
    );
    const mcpBtn = screen.getByRole('button', { name: /mcp servers/i });
    expect(mcpBtn.className).toContain('bg-primary-50');
  });

  it('applies unselected styling to MCP tab when another channel is active', () => {
    renderWithProviders(
      <ChannelSelector
        definitions={FALLBACK_DEFINITIONS}
        selectedChannel="telegram"
        onSelectChannel={onSelect}
      />
    );
    const mcpBtn = screen.getByRole('button', { name: /mcp servers/i });
    expect(mcpBtn.className).not.toContain('bg-primary-50');
    expect(mcpBtn.className).toContain('bg-surface-muted');
  });
});
