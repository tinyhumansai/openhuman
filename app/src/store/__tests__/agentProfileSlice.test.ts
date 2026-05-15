import { configureStore } from '@reduxjs/toolkit';
import { describe, expect, it, vi } from 'vitest';

import type { AgentProfilesResponse } from '../../types/agentProfile';
import reducer, {
  type AgentProfileState,
  deleteAgentProfile,
  loadAgentProfiles,
  selectActiveAgentProfileId,
  selectAgentProfile,
  selectAgentProfiles,
  setAgentProfilesFromResponse,
  upsertAgentProfile,
} from '../agentProfileSlice';
import { resetUserScopedState } from '../resetActions';

const mockList = vi.fn();
const mockSelect = vi.fn();
const mockUpsert = vi.fn();
const mockDelete = vi.fn();

vi.mock('../../services/api/agentProfilesApi', () => ({
  agentProfilesApi: {
    list: (...args: unknown[]) => mockList(...args),
    select: (...args: unknown[]) => mockSelect(...args),
    upsert: (...args: unknown[]) => mockUpsert(...args),
    delete: (...args: unknown[]) => mockDelete(...args),
  },
}));

const twoProfiles: AgentProfilesResponse = {
  activeProfileId: 'planner',
  profiles: [
    {
      id: 'default',
      name: 'Orchestrator',
      description: 'Default',
      agentId: 'orchestrator',
      builtIn: true,
    },
    {
      id: 'planner',
      name: 'Planner',
      description: 'Planning profile',
      agentId: 'planner',
      builtIn: true,
    },
  ],
};

function makeStore() {
  return configureStore({ reducer: { agentProfiles: reducer } });
}

describe('agentProfileSlice', () => {
  it('stores profiles and active profile from backend response', () => {
    const state = reducer(undefined, setAgentProfilesFromResponse(twoProfiles));

    expect(state.activeProfileId).toBe('planner');
    expect(state.profiles).toHaveLength(2);
    expect(state.error).toBeNull();
  });

  it('resets with other user-scoped state', () => {
    const dirty = reducer(undefined, setAgentProfilesFromResponse(twoProfiles));
    const reset = reducer(dirty, resetUserScopedState());

    expect(reset.activeProfileId).toBe('default');
    expect(reset.profiles).toEqual([]);
    expect(reset.status).toBe('idle');
  });

  it('falls back to first profile id when activeProfileId is missing from response', () => {
    const state = reducer(
      undefined,
      setAgentProfilesFromResponse({
        activeProfileId: '',
        profiles: [
          {
            id: 'research',
            name: 'Research',
            description: '',
            agentId: 'researcher',
            builtIn: true,
          },
        ],
      })
    );
    expect(state.activeProfileId).toBe('research');
  });

  it('falls back to "default" when profiles array is empty', () => {
    const state = reducer(
      undefined,
      setAgentProfilesFromResponse({ activeProfileId: '', profiles: [] })
    );
    expect(state.activeProfileId).toBe('default');
  });
});

describe('agentProfileSlice — async thunks', () => {
  it('loadAgentProfiles: pending → loading, fulfilled → idle with profiles', async () => {
    mockList.mockResolvedValueOnce(twoProfiles);
    const store = makeStore();

    const promise = store.dispatch(loadAgentProfiles());
    expect(store.getState().agentProfiles.status).toBe('loading');
    expect(store.getState().agentProfiles.error).toBeNull();

    await promise;
    const state = store.getState().agentProfiles;
    expect(state.status).toBe('idle');
    expect(state.profiles).toHaveLength(2);
    expect(state.activeProfileId).toBe('planner');
    expect(state.error).toBeNull();
  });

  it('loadAgentProfiles: rejected → error with message', async () => {
    mockList.mockRejectedValueOnce(new Error('network error'));
    const store = makeStore();

    await store.dispatch(loadAgentProfiles());
    const state = store.getState().agentProfiles;
    expect(state.status).toBe('error');
    expect(state.error).toContain('network error');
  });

  it('selectAgentProfile: pending → saving, fulfilled → idle with updated active', async () => {
    const updated: AgentProfilesResponse = { ...twoProfiles, activeProfileId: 'default' };
    mockSelect.mockResolvedValueOnce(updated);
    const store = makeStore();

    const promise = store.dispatch(selectAgentProfile('default'));
    expect(store.getState().agentProfiles.status).toBe('saving');

    await promise;
    const state = store.getState().agentProfiles;
    expect(state.status).toBe('idle');
    expect(state.activeProfileId).toBe('default');
  });

  it('selectAgentProfile: rejected → error', async () => {
    mockSelect.mockRejectedValueOnce(new Error('not found'));
    const store = makeStore();

    await store.dispatch(selectAgentProfile('missing'));
    const state = store.getState().agentProfiles;
    expect(state.status).toBe('error');
    expect(state.error).toContain('not found');
  });

  it('upsertAgentProfile: pending → saving, fulfilled → idle', async () => {
    const newProfile = {
      id: 'custom',
      name: 'Custom',
      description: '',
      agentId: 'orchestrator',
      builtIn: false,
    };
    const withCustom: AgentProfilesResponse = {
      activeProfileId: 'default',
      profiles: [...twoProfiles.profiles, newProfile],
    };
    mockUpsert.mockResolvedValueOnce(withCustom);
    const store = makeStore();

    const promise = store.dispatch(upsertAgentProfile(newProfile));
    expect(store.getState().agentProfiles.status).toBe('saving');

    await promise;
    const state = store.getState().agentProfiles;
    expect(state.status).toBe('idle');
    expect(state.profiles).toHaveLength(3);
    expect(state.profiles.some(p => p.id === 'custom')).toBe(true);
  });

  it('upsertAgentProfile: rejected → error', async () => {
    mockUpsert.mockRejectedValueOnce(new Error('validation failed'));
    const store = makeStore();

    await store.dispatch(
      upsertAgentProfile({ id: 'x', name: '', description: '', agentId: '', builtIn: false })
    );
    expect(store.getState().agentProfiles.status).toBe('error');
    expect(store.getState().agentProfiles.error).toContain('validation failed');
  });

  it('deleteAgentProfile: pending → saving, fulfilled → idle with fewer profiles', async () => {
    const afterDelete: AgentProfilesResponse = {
      activeProfileId: 'default',
      profiles: [twoProfiles.profiles[0]],
    };
    mockDelete.mockResolvedValueOnce(afterDelete);
    const store = makeStore();

    const promise = store.dispatch(deleteAgentProfile('planner'));
    expect(store.getState().agentProfiles.status).toBe('saving');

    await promise;
    const state = store.getState().agentProfiles;
    expect(state.status).toBe('idle');
    expect(state.profiles).toHaveLength(1);
  });

  it('deleteAgentProfile: rejected → error', async () => {
    mockDelete.mockRejectedValueOnce(new Error('cannot delete built-in'));
    const store = makeStore();

    await store.dispatch(deleteAgentProfile('default'));
    const state = store.getState().agentProfiles;
    expect(state.status).toBe('error');
    expect(state.error).toContain('cannot delete built-in');
  });
});

describe('agentProfileSlice — selectors', () => {
  it('selectAgentProfiles extracts profiles array', () => {
    const storeState = {
      agentProfiles: {
        profiles: twoProfiles.profiles,
        activeProfileId: 'planner',
        status: 'idle',
        error: null,
      } as AgentProfileState,
    };
    expect(selectAgentProfiles(storeState)).toHaveLength(2);
  });

  it('selectActiveAgentProfileId extracts activeProfileId', () => {
    const storeState = {
      agentProfiles: {
        profiles: [],
        activeProfileId: 'research',
        status: 'idle',
        error: null,
      } as AgentProfileState,
    };
    expect(selectActiveAgentProfileId(storeState)).toBe('research');
  });
});
