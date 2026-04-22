import { screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import { type Capability, listCapabilities } from '../../../../utils/tauriCommands/aboutApp';
import PrivacyPanel from '../PrivacyPanel';

vi.mock('../../../../utils/tauriCommands/aboutApp', () => ({
  listCapabilities: vi.fn(),
}));

vi.mock('../../../../providers/CoreStateProvider', () => ({
  useCoreState: () => ({
    snapshot: { analyticsEnabled: false },
    setAnalyticsEnabled: vi.fn(),
  }),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({ navigateBack: vi.fn(), breadcrumbs: [] }),
}));

const annotated: Capability = {
  id: 'conversation.send_text',
  name: 'Send Text Messages',
  domain: 'conversation',
  category: 'conversation',
  description: 'Send typed messages to the assistant in a conversation.',
  how_to: 'Conversations > Message composer',
  status: 'stable',
  privacy: {
    leaves_device: true,
    data_kind: 'derived',
    destinations: ['OpenHuman backend', 'TinyHumans Neocortex'],
  },
};

const localOnly: Capability = {
  id: 'local_ai.embed_text',
  name: 'Embed Text',
  domain: 'local_ai',
  category: 'local_ai',
  description: 'Generate embeddings locally.',
  how_to: 'Local AI',
  status: 'stable',
  privacy: { leaves_device: false, data_kind: 'raw', destinations: [] },
};

const unannotated: Capability = {
  id: 'conversation.create',
  name: 'Create Conversations',
  domain: 'conversation',
  category: 'conversation',
  description: 'Start a new conversation thread.',
  how_to: 'Conversations',
  status: 'stable',
};

describe('PrivacyPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders annotated capabilities returned by about_app.list', async () => {
    vi.mocked(listCapabilities).mockResolvedValue([annotated, localOnly]);
    renderWithProviders(<PrivacyPanel />);

    await waitFor(() => {
      expect(screen.getByTestId('privacy-capability-list')).toBeTruthy();
    });

    expect(screen.getByTestId('privacy-row-conversation.send_text')).toBeTruthy();
    expect(screen.getByTestId('privacy-row-local_ai.embed_text')).toBeTruthy();
    expect(screen.getByText(/OpenHuman backend, TinyHumans Neocortex/)).toBeTruthy();
    expect(screen.getByText('Stays local')).toBeTruthy();
  });

  it('omits capabilities without privacy metadata', async () => {
    vi.mocked(listCapabilities).mockResolvedValue([annotated, unannotated]);
    renderWithProviders(<PrivacyPanel />);

    await waitFor(() => {
      expect(screen.getByTestId('privacy-row-conversation.send_text')).toBeTruthy();
    });
    expect(screen.queryByTestId('privacy-row-conversation.create')).toBeNull();
  });

  it('shows graceful fallback when the RPC fails and keeps analytics toggle visible', async () => {
    vi.mocked(listCapabilities).mockRejectedValue(new Error('boom'));
    renderWithProviders(<PrivacyPanel />);

    await waitFor(() => {
      expect(screen.getByTestId('privacy-load-error')).toBeTruthy();
    });
    expect(screen.queryByTestId('privacy-capability-list')).toBeNull();
    // Analytics toggle still rendered
    expect(screen.getByRole('switch')).toBeTruthy();
  });
});
