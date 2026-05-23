/**
 * Tests for ChannelConfigPanel — covers the MCP virtual tab and the
 * channel-definition-backed tabs (telegram, discord, web) and the null
 * fallback when no matching definition is found.
 */
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { FALLBACK_DEFINITIONS } from '../../../lib/channels/definitions';
import { renderWithProviders } from '../../../test/test-utils';
import ChannelConfigPanel from '../ChannelConfigPanel';

// McpServersTab is a heavy async component — mock it so ChannelConfigPanel
// tests stay focused on the routing logic (line 16 branch).
vi.mock('../mcp/McpServersTab', () => ({
  default: () => <div data-testid="mcp-servers-tab">MCP Servers Tab</div>,
}));

// Mock channel-specific config panels to keep tests lightweight.
vi.mock('../TelegramConfig', () => ({
  default: () => <div data-testid="telegram-config">Telegram Config</div>,
}));

vi.mock('../DiscordConfig', () => ({
  default: () => <div data-testid="discord-config">Discord Config</div>,
}));

vi.mock('../WebChannelConfig', () => ({
  default: () => <div data-testid="web-config">Web Config</div>,
}));

vi.mock('../ChannelCapabilities', () => ({
  default: () => <div data-testid="channel-capabilities">Capabilities</div>,
}));

describe('ChannelConfigPanel', () => {
  it('renders McpServersTab when selectedChannel is "mcp"', () => {
    render(<ChannelConfigPanel selectedChannel="mcp" definitions={FALLBACK_DEFINITIONS} />);
    expect(screen.getByTestId('mcp-servers-tab')).toBeInTheDocument();
    expect(screen.getByText('MCP Servers')).toBeInTheDocument();
  });

  it('does not render definition-based content when channel is "mcp"', () => {
    render(<ChannelConfigPanel selectedChannel="mcp" definitions={FALLBACK_DEFINITIONS} />);
    // No Telegram/Discord/Web-specific config panels
    expect(screen.queryByTestId('telegram-config')).not.toBeInTheDocument();
    expect(screen.queryByTestId('discord-config')).not.toBeInTheDocument();
  });

  it('renders TelegramConfig when selectedChannel is "telegram"', () => {
    renderWithProviders(
      <ChannelConfigPanel selectedChannel="telegram" definitions={FALLBACK_DEFINITIONS} />
    );
    expect(screen.getByTestId('telegram-config')).toBeInTheDocument();
  });

  it('renders DiscordConfig when selectedChannel is "discord"', () => {
    renderWithProviders(
      <ChannelConfigPanel selectedChannel="discord" definitions={FALLBACK_DEFINITIONS} />
    );
    expect(screen.getByTestId('discord-config')).toBeInTheDocument();
  });

  it('renders channel display_name and description for a matched definition', () => {
    renderWithProviders(
      <ChannelConfigPanel selectedChannel="telegram" definitions={FALLBACK_DEFINITIONS} />
    );
    expect(screen.getByText('Telegram')).toBeInTheDocument();
    expect(screen.getByText(/send and receive messages via telegram/i)).toBeInTheDocument();
  });

  it('renders nothing when selectedChannel has no matching definition', () => {
    const { container } = renderWithProviders(
      // 'mcp' is handled above; use an unknown channel to hit the null-return
      // branch (definition not found).
      <ChannelConfigPanel selectedChannel={'unknown' as never} definitions={[]} />
    );
    expect(container.firstChild).toBeNull();
  });
});
