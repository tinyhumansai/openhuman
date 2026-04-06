import { screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { FALLBACK_DEFINITIONS } from '../../lib/channels/definitions';
import { renderWithProviders } from '../../test/test-utils';
import Channels from '../Channels';

vi.mock('../../hooks/useChannelDefinitions', () => ({
  useChannelDefinitions: () => ({
    definitions: FALLBACK_DEFINITIONS,
    loading: false,
    error: null,
    refreshDefinitions: vi.fn(),
  }),
}));

describe('Channels page', () => {
  it('renders the channel selector after loading', async () => {
    renderWithProviders(<Channels />);

    expect((await screen.findAllByText('Telegram')).length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText('Discord').length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText('Web').length).toBeGreaterThanOrEqual(1);
  });

  it('renders the Telegram config panel by default', async () => {
    renderWithProviders(<Channels />);

    expect(await screen.findByText('Login with OpenHuman')).toBeInTheDocument();
  });
});
