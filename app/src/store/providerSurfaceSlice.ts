import { createAsyncThunk, createSlice } from '@reduxjs/toolkit';

import { providerSurfacesApi } from '../services/api/providerSurfacesApi';
import type { RespondQueueItem } from '../types/providerSurfaces';
import { resetUserScopedState } from './resetActions';

interface ProviderSurfaceState {
  queue: RespondQueueItem[];
  count: number;
  status: 'idle' | 'loading' | 'succeeded' | 'failed';
  error: string | null;
  lastSyncedAt: number | null;
}

const initialState: ProviderSurfaceState = {
  queue: [],
  count: 0,
  status: 'idle',
  error: null,
  lastSyncedAt: null,
};

/** Pass `{ silent: true }` for background refresh (no loading flicker). */
export const fetchRespondQueue = createAsyncThunk(
  'providerSurfaces/fetchRespondQueue',
  async (_options: { silent?: boolean } | undefined, { rejectWithValue }) => {
    try {
      return await providerSurfacesApi.listQueue();
    } catch (error) {
      return rejectWithValue(
        error instanceof Error ? error.message : 'Failed to load provider respond queue'
      );
    }
  }
);

const providerSurfaceSlice = createSlice({
  name: 'providerSurfaces',
  initialState,
  reducers: {},
  extraReducers: builder => {
    builder
      .addCase(fetchRespondQueue.pending, (state, action) => {
        if (!action.meta.arg?.silent) {
          state.status = 'loading';
          state.error = null;
        }
      })
      .addCase(fetchRespondQueue.fulfilled, (state, action) => {
        state.status = 'succeeded';
        state.queue = action.payload.items;
        state.count = action.payload.count;
        state.lastSyncedAt = Date.now();
      })
      .addCase(fetchRespondQueue.rejected, (state, action) => {
        if (!action.meta.arg?.silent) {
          state.status = 'failed';
          state.error = (action.payload as string) ?? 'Failed to load provider respond queue';
        }
        // silent failures: leave status/error as-is; a subsequent successful poll will clear
      })
      .addCase(resetUserScopedState, () => initialState);
  },
});

export default providerSurfaceSlice.reducer;
