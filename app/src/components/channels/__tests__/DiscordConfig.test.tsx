import { screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { FALLBACK_DEFINITIONS } from '../../../lib/channels/definitions';
import { renderWithProviders } from '../../../test/test-utils';
import DiscordConfig from '../DiscordConfig';

const coreStateMock = vi.hoisted(() => vi.fn(() => ({ snapshot: { sessionToken: 'jwt-abc' } })));

vi.mock('../../../providers/CoreStateProvider', () => ({ useCoreState: () => coreStateMock() }));

const discordDef = FALLBACK_DEFINITIONS.find(d => d.id === 'discord')!;

afterEach(() => {
  vi.clearAllMocks();
  coreStateMock.mockReturnValue({ snapshot: { sessionToken: 'jwt-abc' } });
});

describe('DiscordConfig', () => {
  it('renders auth mode labels', () => {
    renderWithProviders(<DiscordConfig definition={discordDef} />);
    expect(screen.getByText('OAuth Sign-in')).toBeInTheDocument();
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
    expect(connectButtons.length).toBe(3);
  });

  it('hides managed channel auth modes for local users', () => {
    coreStateMock.mockReturnValue({ snapshot: { sessionToken: 'header.payload.local' } });

    renderWithProviders(<DiscordConfig definition={discordDef} />);

    expect(
      screen.getByText('Managed channels are not available for local users.')
    ).toBeInTheDocument();
    expect(screen.queryByText('OAuth Sign-in')).not.toBeInTheDocument();
    expect(screen.queryByText('Login with OpenHuman')).not.toBeInTheDocument();
    expect(screen.getAllByText('Bot Token').length).toBeGreaterThanOrEqual(1);
  });
});
