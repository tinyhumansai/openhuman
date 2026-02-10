import { createAsyncThunk, createSlice } from '@reduxjs/toolkit';

import { teamApi } from '../services/api/teamApi';
import type { TeamInvite, TeamMember, TeamWithRole } from '../types/team';

interface TeamState {
  teams: TeamWithRole[];
  members: TeamMember[];
  invites: TeamInvite[];
  isLoading: boolean;
  isLoadingMembers: boolean;
  isLoadingInvites: boolean;
  error: string | null;
}

const initialState: TeamState = {
  teams: [],
  members: [],
  invites: [],
  isLoading: false,
  isLoadingMembers: false,
  isLoadingInvites: false,
  error: null,
};

export const fetchTeams = createAsyncThunk('team/fetchTeams', async (_, { rejectWithValue }) => {
  try {
    return await teamApi.getTeams();
  } catch (error) {
    const msg =
      error && typeof error === 'object' && 'error' in error
        ? String(error.error)
        : 'Failed to fetch teams';
    return rejectWithValue(msg);
  }
});

export const fetchMembers = createAsyncThunk(
  'team/fetchMembers',
  async (teamId: string, { rejectWithValue }) => {
    try {
      return await teamApi.getMembers(teamId);
    } catch (error) {
      const msg =
        error && typeof error === 'object' && 'error' in error
          ? String(error.error)
          : 'Failed to fetch members';
      return rejectWithValue(msg);
    }
  }
);

export const fetchInvites = createAsyncThunk(
  'team/fetchInvites',
  async (teamId: string, { rejectWithValue }) => {
    try {
      return await teamApi.getInvites(teamId);
    } catch (error) {
      const msg =
        error && typeof error === 'object' && 'error' in error
          ? String(error.error)
          : 'Failed to fetch invites';
      return rejectWithValue(msg);
    }
  }
);

const teamSlice = createSlice({
  name: 'team',
  initialState,
  reducers: { clearTeamState: () => initialState },
  extraReducers: builder => {
    builder
      // fetchTeams
      .addCase(fetchTeams.pending, state => {
        state.isLoading = true;
        state.error = null;
      })
      .addCase(fetchTeams.fulfilled, (state, action) => {
        state.isLoading = false;
        state.teams = action.payload;
      })
      .addCase(fetchTeams.rejected, (state, action) => {
        state.isLoading = false;
        state.error = action.payload as string;
      })
      // fetchMembers
      .addCase(fetchMembers.pending, state => {
        state.isLoadingMembers = true;
        state.error = null;
      })
      .addCase(fetchMembers.fulfilled, (state, action) => {
        state.isLoadingMembers = false;
        state.members = action.payload;
      })
      .addCase(fetchMembers.rejected, (state, action) => {
        state.isLoadingMembers = false;
        state.error = action.payload as string;
      })
      // fetchInvites
      .addCase(fetchInvites.pending, state => {
        state.isLoadingInvites = true;
        state.error = null;
      })
      .addCase(fetchInvites.fulfilled, (state, action) => {
        state.isLoadingInvites = false;
        state.invites = action.payload;
      })
      .addCase(fetchInvites.rejected, (state, action) => {
        state.isLoadingInvites = false;
        state.error = action.payload as string;
      });
  },
});

export const { clearTeamState } = teamSlice.actions;
export default teamSlice.reducer;
