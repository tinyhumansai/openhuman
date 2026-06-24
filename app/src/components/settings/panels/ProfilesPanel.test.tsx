import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { agentProfilesApi } from '../../../services/api/agentProfilesApi';
import agentProfilesReducer from '../../../store/agentProfileSlice';
import type { AgentProfile, AgentProfilesResponse } from '../../../types/agentProfile';
import ProfilesPanel from './ProfilesPanel';

vi.mock('../../../services/api/agentProfilesApi', () => ({
  agentProfilesApi: { list: vi.fn(), select: vi.fn(), upsert: vi.fn(), delete: vi.fn() },
}));

const mockNavigate = vi.fn();
vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => mockNavigate };
});

vi.mock('../components/SettingsHeader', () => ({
  default: ({ title, action }: { title: string; action?: React.ReactNode }) => (
    <div>
      <h1>{title}</h1>
      {action}
    </div>
  ),
}));

const mockList = vi.mocked(agentProfilesApi.list);
const mockSelect = vi.mocked(agentProfilesApi.select);
const mockDelete = vi.mocked(agentProfilesApi.delete);

function profile(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id: 'research',
    name: 'Research',
    description: 'Source-grounded research.',
    agentId: 'researcher',
    builtIn: true,
    ...overrides,
  };
}

function response(profiles: AgentProfile[], activeProfileId: string): AgentProfilesResponse {
  return { profiles, activeProfileId };
}

const PROFILES = [
  profile({ id: 'default', name: 'Default', builtIn: true }),
  profile({ id: 'research', name: 'Research', builtIn: true }),
  profile({ id: 'writer', name: 'Writer', description: 'Drafts copy.', builtIn: false }),
];

function renderPanel() {
  const store = configureStore({ reducer: { agentProfiles: agentProfilesReducer } });
  return {
    store,
    ...render(
      <Provider store={store}>
        <MemoryRouter>
          <ProfilesPanel />
        </MemoryRouter>
      </Provider>
    ),
  };
}

describe('ProfilesPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockList.mockResolvedValue(response(PROFILES, 'default'));
  });

  it('loads and lists profiles with active + source badges', async () => {
    renderPanel();
    expect(await screen.findByText('Writer')).toBeInTheDocument();
    expect(screen.getByText('Research')).toBeInTheDocument();
    // Active badge on the default profile, source badges on all.
    expect(screen.getByText('Active')).toBeInTheDocument();
    expect(screen.getAllByText('Built-in').length).toBeGreaterThan(0);
    expect(screen.getByText('Custom')).toBeInTheDocument();
    expect(mockList).toHaveBeenCalled();
  });

  it('navigates to the create page from the New profile action', async () => {
    renderPanel();
    await screen.findByText('Writer');
    fireEvent.click(screen.getByText('New profile'));
    // Second arg carries the backgroundLocation nav state for the desktop modal.
    expect(mockNavigate).toHaveBeenCalledWith('/settings/profiles/new', expect.anything());
  });

  it('sets a non-active profile as active', async () => {
    mockSelect.mockResolvedValue(response(PROFILES, 'writer'));
    renderPanel();
    await screen.findByText('Writer');
    // "Set as active" is shown for non-active profiles.
    fireEvent.click(screen.getAllByText('Set as active')[0]);
    await waitFor(() => expect(mockSelect).toHaveBeenCalled());
  });

  it('navigates to the edit page for a profile', async () => {
    renderPanel();
    await screen.findByText('Writer');
    fireEvent.click(screen.getAllByText('Edit')[0]);
    expect(mockNavigate).toHaveBeenCalledWith(
      expect.stringContaining('/settings/profiles/edit/'),
      expect.anything()
    );
  });

  it('deletes a custom profile after confirmation', async () => {
    mockDelete.mockResolvedValue(response(PROFILES.slice(0, 2), 'default'));
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);
    renderPanel();
    await screen.findByText('Writer');
    // Only the custom profile (Writer) has a Delete button.
    fireEvent.click(screen.getByText('Delete'));
    await waitFor(() => expect(mockDelete).toHaveBeenCalledWith('writer'));
    confirmSpy.mockRestore();
  });

  it('does not delete when confirmation is cancelled', async () => {
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(false);
    renderPanel();
    await screen.findByText('Writer');
    fireEvent.click(screen.getByText('Delete'));
    expect(mockDelete).not.toHaveBeenCalled();
    confirmSpy.mockRestore();
  });

  it('renders the empty state when there are no profiles', async () => {
    mockList.mockResolvedValue(response([], 'default'));
    renderPanel();
    expect(await screen.findByText('No agent profiles yet')).toBeInTheDocument();
  });

  it('surfaces a load error', async () => {
    mockList.mockRejectedValue(new Error('boom'));
    renderPanel();
    await waitFor(() => expect(screen.getByText(/boom/)).toBeInTheDocument());
  });
});
