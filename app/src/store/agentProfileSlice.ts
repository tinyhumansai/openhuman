import { createAsyncThunk, createSlice, type PayloadAction } from '@reduxjs/toolkit';
import debug from 'debug';

import { agentProfilesApi } from '../services/api/agentProfilesApi';
import type { AgentProfile, AgentProfilesResponse } from '../types/agentProfile';
import { resetUserScopedState } from './resetActions';

const log = debug('agentProfiles');

export type AgentProfilesStatus = 'idle' | 'loading' | 'saving' | 'error';

export interface AgentProfileState {
  profiles: AgentProfile[];
  activeProfileId: string;
  status: AgentProfilesStatus;
  error: string | null;
}

const initialState: AgentProfileState = {
  profiles: [],
  activeProfileId: 'default',
  status: 'idle',
  error: null,
};

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}

function applyProfileResponse(state: AgentProfileState, response: AgentProfilesResponse) {
  state.profiles = response.profiles;
  state.activeProfileId = response.activeProfileId || response.profiles[0]?.id || 'default';
  state.error = null;
}

export const loadAgentProfiles = createAsyncThunk('agentProfiles/load', async () =>
  agentProfilesApi.list()
);

export const selectAgentProfile = createAsyncThunk(
  'agentProfiles/select',
  async (profileId: string) => agentProfilesApi.select(profileId)
);

export const upsertAgentProfile = createAsyncThunk(
  'agentProfiles/upsert',
  async (profile: AgentProfile) => agentProfilesApi.upsert(profile)
);

export const deleteAgentProfile = createAsyncThunk(
  'agentProfiles/delete',
  async (profileId: string) => agentProfilesApi.delete(profileId)
);

const agentProfileSlice = createSlice({
  name: 'agentProfiles',
  initialState,
  reducers: {
    setAgentProfilesFromResponse(state, action: PayloadAction<AgentProfilesResponse>) {
      applyProfileResponse(state, action.payload);
      state.status = 'idle';
    },
  },
  extraReducers: builder => {
    builder
      .addCase(loadAgentProfiles.pending, state => {
        state.status = 'loading';
        state.error = null;
      })
      .addCase(loadAgentProfiles.fulfilled, (state, action) => {
        applyProfileResponse(state, action.payload);
        state.status = 'idle';
        log('loaded %d profile(s), active=%s', state.profiles.length, state.activeProfileId);
      })
      .addCase(loadAgentProfiles.rejected, (state, action) => {
        state.status = 'error';
        state.error = errorMessage(action.error.message ?? action.error);
      })
      .addCase(selectAgentProfile.pending, state => {
        state.status = 'saving';
        state.error = null;
      })
      .addCase(selectAgentProfile.fulfilled, (state, action) => {
        applyProfileResponse(state, action.payload);
        state.status = 'idle';
      })
      .addCase(selectAgentProfile.rejected, (state, action) => {
        state.status = 'error';
        state.error = errorMessage(action.error.message ?? action.error);
      })
      .addCase(upsertAgentProfile.pending, state => {
        state.status = 'saving';
        state.error = null;
      })
      .addCase(upsertAgentProfile.fulfilled, (state, action) => {
        applyProfileResponse(state, action.payload);
        state.status = 'idle';
      })
      .addCase(upsertAgentProfile.rejected, (state, action) => {
        state.status = 'error';
        state.error = errorMessage(action.error.message ?? action.error);
      })
      .addCase(deleteAgentProfile.pending, state => {
        state.status = 'saving';
        state.error = null;
      })
      .addCase(deleteAgentProfile.fulfilled, (state, action) => {
        applyProfileResponse(state, action.payload);
        state.status = 'idle';
      })
      .addCase(deleteAgentProfile.rejected, (state, action) => {
        state.status = 'error';
        state.error = errorMessage(action.error.message ?? action.error);
      })
      .addCase(resetUserScopedState, () => initialState);
  },
});

export const { setAgentProfilesFromResponse } = agentProfileSlice.actions;

export const selectAgentProfiles = (state: { agentProfiles: AgentProfileState }) =>
  state.agentProfiles.profiles;

export const selectActiveAgentProfileId = (state: { agentProfiles: AgentProfileState }) =>
  state.agentProfiles.activeProfileId;

export default agentProfileSlice.reducer;
