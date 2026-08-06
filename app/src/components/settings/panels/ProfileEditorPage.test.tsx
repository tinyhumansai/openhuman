import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { agentProfilesApi } from '../../../services/api/agentProfilesApi';
import agentProfilesReducer from '../../../store/agentProfileSlice';
import type { AgentProfile } from '../../../types/agentProfile';
import ProfileEditorPage from './ProfileEditorPage';

vi.mock('../../../services/api/agentProfilesApi', () => ({
  agentProfilesApi: { list: vi.fn(), select: vi.fn(), upsert: vi.fn(), delete: vi.fn() },
}));

const mockNavigate = vi.fn();
vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => mockNavigate };
});

const mockUpsert = vi.mocked(agentProfilesApi.upsert);

function profile(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id: 'writer',
    name: 'Writer',
    description: 'Drafts copy.',
    agentId: 'orchestrator',
    builtIn: false,
    ...overrides,
  };
}

function renderAt(path: string, profiles: AgentProfile[] = []) {
  const store = configureStore({
    reducer: { agentProfiles: agentProfilesReducer },
    preloadedState: {
      agentProfiles: { profiles, activeProfileId: 'default', status: 'idle' as const, error: null },
    },
  });
  return {
    store,
    ...render(
      <Provider store={store}>
        <MemoryRouter initialEntries={[path]}>
          <Routes>
            <Route path="/settings/profiles/new" element={<ProfileEditorPage />} />
            <Route path="/settings/profiles/edit/:id" element={<ProfileEditorPage />} />
          </Routes>
        </MemoryRouter>
      </Provider>
    ),
  };
}

describe('ProfileEditorPage', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockUpsert.mockResolvedValue({ profiles: [], activeProfileId: 'default' });
  });

  it('create mode: name drives the slug and Create dispatches an upsert', async () => {
    renderAt('/settings/profiles/new');

    const name = screen.getByLabelText('Name');
    expect(name).toBeInTheDocument();
    fireEvent.change(name, { target: { value: 'My Research' } });
    const id = screen.getByLabelText('ID') as HTMLInputElement;
    expect(id.value).toBe('my-research'); // auto-slugged

    fireEvent.click(screen.getByText('Create'));
    await waitFor(() => expect(mockUpsert).toHaveBeenCalled());
    const sent = mockUpsert.mock.calls[0][0];
    expect(sent.id).toBe('my-research');
    expect(sent.name).toBe('My Research');
    expect(sent.includeAgentConversations).toBe(true);
    expect(mockNavigate).toHaveBeenCalledWith('/settings/profiles', expect.anything());
  });

  it('disables Create until a non-empty resolved id exists', () => {
    renderAt('/settings/profiles/new');
    const create = screen.getByText('Create').closest('button')!;
    expect(create).toBeDisabled();
    // A punctuation-only name still slugs to '' → stays disabled.
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: '!!!' } });
    expect(create).toBeDisabled();
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Ok' } });
    expect(create).not.toBeDisabled();
  });

  it('edit mode hydrates fields from the existing profile', () => {
    renderAt('/settings/profiles/edit/writer', [
      profile({
        id: 'writer',
        name: 'Writer',
        description: 'Drafts copy.',
        soulMd: 'I am Writer.',
      }),
    ]);
    expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Writer');
    expect((screen.getByLabelText('Description') as HTMLTextAreaElement).value).toBe(
      'Drafts copy.'
    );
    expect((screen.getByLabelText('Soul (SOUL.md)') as HTMLTextAreaElement).value).toBe(
      'I am Writer.'
    );
  });

  it('shows not-found when editing an id absent from a loaded list', () => {
    renderAt('/settings/profiles/edit/ghost', [profile({ id: 'writer' })]);
    expect(screen.getByText('Profile not found')).toBeInTheDocument();
  });

  it('an All/Selected allowlist accepts a typed chip', () => {
    renderAt('/settings/profiles/new');
    // Switch the Skills allowlist from All to Selected.
    const selectedButtons = screen.getAllByText('Selected');
    fireEvent.click(selectedButtons[0]);
    const chipInput = screen.getByPlaceholderText('Type an id, press Enter');
    fireEvent.change(chipInput, { target: { value: 'deep-research' } });
    fireEvent.keyDown(chipInput, { key: 'Enter' });
    expect(screen.getByText('deep-research')).toBeInTheDocument();

    // Switching back to All clears the restriction (exercises the All button's
    // onChange(null) handler).
    fireEvent.click(screen.getAllByText('All')[0]);
    expect(screen.queryByPlaceholderText('Type an id, press Enter')).not.toBeInTheDocument();
  });

  it('toggles the recall-agent-conversations switch into the saved payload', async () => {
    renderAt('/settings/profiles/new');
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Scoped' } });
    fireEvent.click(screen.getByLabelText('Recall agent conversations')); // true -> false
    fireEvent.click(screen.getByText('Create'));
    await waitFor(() => expect(mockUpsert).toHaveBeenCalled());
    expect(mockUpsert.mock.calls[0][0].includeAgentConversations).toBe(false);
  });

  it('defaults the dedicated memory/workspace toggles to off and dispatches them true when flipped', async () => {
    renderAt('/settings/profiles/new');
    const dedicatedMemory = screen.getByLabelText('Dedicated memory');
    const dedicatedWorkspace = screen.getByLabelText('Dedicated workspace');
    expect(dedicatedMemory).toHaveAttribute('aria-checked', 'false');
    expect(dedicatedWorkspace).toHaveAttribute('aria-checked', 'false');

    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Isolated' } });
    fireEvent.click(dedicatedMemory);
    fireEvent.click(dedicatedWorkspace);
    expect(screen.getByLabelText('Dedicated memory')).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByLabelText('Dedicated workspace')).toHaveAttribute('aria-checked', 'true');
    fireEvent.click(screen.getByText('Create'));

    await waitFor(() => expect(mockUpsert).toHaveBeenCalled());
    expect(mockUpsert.mock.calls[0][0].dedicatedMemory).toBe(true);
    expect(mockUpsert.mock.calls[0][0].dedicatedWorkspace).toBe(true);
  });

  it('edit mode hydrates the dedicated toggles and shows the resolved read-only paths', () => {
    renderAt('/settings/profiles/edit/writer', [
      profile({
        id: 'writer',
        name: 'Writer',
        dedicatedMemory: true,
        dedicatedWorkspace: true,
        soulMdFile: '/workspace/personalities/writer/SOUL.md',
        workspaceDir: '/action/profiles/writer',
      }),
    ]);
    expect(screen.getByLabelText('Dedicated memory')).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByLabelText('Dedicated workspace')).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByText('/workspace/personalities/writer/SOUL.md')).toBeInTheDocument();
    expect(screen.getByText('/action/profiles/writer')).toBeInTheDocument();
  });

  it('hides the resolved read-only path rows when the profile has none', () => {
    renderAt('/settings/profiles/edit/writer', [profile({ id: 'writer' })]);
    expect(screen.queryByText('SOUL.md file')).not.toBeInTheDocument();
    expect(screen.queryByText('Workspace directory')).not.toBeInTheDocument();
    expect(screen.queryByText('Skills directory')).not.toBeInTheDocument();
  });

  it('shows the resolved skills directory path and hint when present', () => {
    renderAt('/settings/profiles/edit/writer', [
      profile({
        id: 'writer',
        name: 'Writer',
        skillsDir: '/workspace/personalities/writer/skills',
      }),
    ]);
    expect(screen.getByText('Skills directory')).toBeInTheDocument();
    expect(screen.getByText('/workspace/personalities/writer/skills')).toBeInTheDocument();
    expect(
      screen.getByText('SKILL.md files placed here are private to this profile.')
    ).toBeInTheDocument();
  });
});
