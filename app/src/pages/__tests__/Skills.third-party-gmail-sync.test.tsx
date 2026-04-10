/**
 * Skills page — 3rd Party Skills: manual sync triggers core `openhuman.skills_sync`
 * via `skillManager.triggerSync` (see `lib/skills/manager.ts`).
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import Skills from '../Skills';

const gmailRegistryEntry = {
  id: 'gmail',
  name: 'Gmail',
  version: '1.1.0',
  description: 'Read and send email via Gmail.',
  runtime: 'quickjs',
  entry: 'index.js',
  auto_start: false,
  platforms: ['macos', 'linux', 'windows'],
  setup: { required: true, label: 'Connect Gmail' },
};

const mocks = vi.hoisted(() => ({
  triggerSync: vi.fn().mockResolvedValue(undefined),
  startSkill: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../hooks/useChannelDefinitions', () => ({
  useChannelDefinitions: () => ({ definitions: [], loading: false, error: null }),
}));

vi.mock('../../lib/skills/manager', () => ({
  skillManager: { triggerSync: mocks.triggerSync, startSkill: mocks.startSkill },
}));

vi.mock('../../lib/skills/skillsApi', () => ({
  installSkill: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../lib/skills/hooks', () => ({
  useAvailableSkills: () => ({ skills: [gmailRegistryEntry], loading: false, refresh: vi.fn() }),
  useSkillConnectionStatus: (skillId: string) => (skillId === 'gmail' ? 'connected' : 'offline'),
  useSkillState: () => ({
    connection_status: 'connected',
    auth_status: 'authenticated',
    syncInProgress: false,
    syncProgress: 0,
    syncProgressMessage: '',
  }),
  useSkillDataDirectoryStats: () => undefined,
}));

describe('Skills page — 3rd Party Gmail sync', () => {
  beforeEach(() => {
    mocks.triggerSync.mockClear();
    mocks.startSkill.mockClear();
  });

  it('renders 3rd Party Skills and Gmail with a Sync control when connected', async () => {
    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });

    expect(screen.getByText('Gmail')).toBeInTheDocument();

    // Sync button is inside the overflow menu — open it first
    const moreBtn = screen.getByTitle('More actions');
    fireEvent.click(moreBtn);

    const syncBtn = await screen.findByTestId('skill-sync-button-gmail');
    expect(syncBtn).toBeInTheDocument();

    fireEvent.click(syncBtn);

    await waitFor(() => {
      expect(mocks.triggerSync).toHaveBeenCalledTimes(1);
    });
    expect(mocks.triggerSync).toHaveBeenCalledWith('gmail');
  });
});
