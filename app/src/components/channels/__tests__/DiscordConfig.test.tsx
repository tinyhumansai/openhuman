import { screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { FALLBACK_DEFINITIONS } from '../../../lib/channels/definitions';
import { renderWithProviders } from '../../../test/test-utils';
import DiscordConfig from '../DiscordConfig';

const discordDef = FALLBACK_DEFINITIONS.find(d => d.id === 'discord')!;

describe('DiscordConfig', () => {
  it('renders the Discord header', () => {
    renderWithProviders(<DiscordConfig definition={discordDef} />);
    expect(screen.getByText('Discord')).toBeInTheDocument();
  });

  it('renders both auth modes', () => {
    renderWithProviders(<DiscordConfig definition={discordDef} />);
    expect(screen.getAllByText('Bot Token').length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText('OAuth Sign-in')).toBeInTheDocument();
  });

  it('shows credential fields for bot_token mode', () => {
    renderWithProviders(<DiscordConfig definition={discordDef} />);
    expect(screen.getByPlaceholderText(/Your Discord bot token/)).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/restrict to a specific server/)).toBeInTheDocument();
  });

  it('shows Connect buttons for each auth mode', () => {
    renderWithProviders(<DiscordConfig definition={discordDef} />);
    const connectButtons = screen.getAllByText('Connect');
    expect(connectButtons.length).toBe(2);
  });
});
