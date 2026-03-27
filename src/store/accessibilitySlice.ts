import { createAsyncThunk, createSlice, type PayloadAction } from '@reduxjs/toolkit';

import {
  type AccessibilityInputActionParams,
  type AccessibilitySessionStatus,
  type AccessibilityStatus,
  openhumanAccessibilityInputAction,
  openhumanAccessibilityRequestPermissions,
  openhumanAccessibilityStartSession,
  openhumanAccessibilityStatus,
  openhumanAccessibilityStopSession,
} from '../utils/tauriCommands';

interface AccessibilityState {
  status: AccessibilityStatus | null;
  isLoading: boolean;
  isRequestingPermissions: boolean;
  isStartingSession: boolean;
  isStoppingSession: boolean;
  lastError: string | null;
}

const initialState: AccessibilityState = {
  status: null,
  isLoading: false,
  isRequestingPermissions: false,
  isStartingSession: false,
  isStoppingSession: false,
  lastError: null,
};

const extractError = (error: unknown, fallback: string): string => {
  if (error instanceof Error && error.message.trim()) {
    return error.message;
  }
  return fallback;
};

export const fetchAccessibilityStatus = createAsyncThunk(
  'accessibility/fetchStatus',
  async (_, { rejectWithValue }) => {
    try {
      const response = await openhumanAccessibilityStatus();
      return response.result;
    } catch (error) {
      return rejectWithValue(extractError(error, 'Failed to fetch accessibility status'));
    }
  }
);

export const requestAccessibilityPermissions = createAsyncThunk(
  'accessibility/requestPermissions',
  async (_, { rejectWithValue }) => {
    try {
      await openhumanAccessibilityRequestPermissions();
      const response = await openhumanAccessibilityStatus();
      return response.result;
    } catch (error) {
      return rejectWithValue(extractError(error, 'Failed to request accessibility permissions'));
    }
  }
);

export const startAccessibilitySession = createAsyncThunk(
  'accessibility/startSession',
  async (
    params: {
      consent: boolean;
      ttl_secs?: number;
      screen_monitoring?: boolean;
      device_control?: boolean;
      predictive_input?: boolean;
    },
    { rejectWithValue }
  ) => {
    try {
      await openhumanAccessibilityStartSession(params);
      const response = await openhumanAccessibilityStatus();
      return response.result;
    } catch (error) {
      return rejectWithValue(extractError(error, 'Failed to start accessibility session'));
    }
  }
);

export const stopAccessibilitySession = createAsyncThunk(
  'accessibility/stopSession',
  async (reason: string | undefined, { rejectWithValue }) => {
    try {
      await openhumanAccessibilityStopSession(reason ? { reason } : undefined);
      const response = await openhumanAccessibilityStatus();
      return response.result;
    } catch (error) {
      return rejectWithValue(extractError(error, 'Failed to stop accessibility session'));
    }
  }
);

export const executeAccessibilityInputAction = createAsyncThunk(
  'accessibility/inputAction',
  async (params: AccessibilityInputActionParams, { rejectWithValue }) => {
    try {
      const response = await openhumanAccessibilityInputAction(params);
      return response.result;
    } catch (error) {
      return rejectWithValue(extractError(error, 'Failed to execute input action'));
    }
  }
);

const accessibilitySlice = createSlice({
  name: 'accessibility',
  initialState,
  reducers: {
    clearAccessibilityError(state) {
      state.lastError = null;
    },
    setAccessibilityStatus(state, action: PayloadAction<AccessibilityStatus | null>) {
      state.status = action.payload;
    },
    setAccessibilitySessionFeatures(state, action: PayloadAction<AccessibilitySessionStatus>) {
      if (state.status) {
        state.status.session = action.payload;
      }
    },
  },
  extraReducers: builder => {
    builder
      .addCase(fetchAccessibilityStatus.pending, state => {
        state.isLoading = true;
        state.lastError = null;
      })
      .addCase(fetchAccessibilityStatus.fulfilled, (state, action) => {
        state.isLoading = false;
        state.status = action.payload;
      })
      .addCase(fetchAccessibilityStatus.rejected, (state, action) => {
        state.isLoading = false;
        state.lastError = (action.payload as string) ?? 'Failed to fetch accessibility status';
      })
      .addCase(requestAccessibilityPermissions.pending, state => {
        state.isRequestingPermissions = true;
        state.lastError = null;
      })
      .addCase(requestAccessibilityPermissions.fulfilled, (state, action) => {
        state.isRequestingPermissions = false;
        state.status = action.payload;
      })
      .addCase(requestAccessibilityPermissions.rejected, (state, action) => {
        state.isRequestingPermissions = false;
        state.lastError =
          (action.payload as string) ?? 'Failed to request accessibility permissions';
      })
      .addCase(startAccessibilitySession.pending, state => {
        state.isStartingSession = true;
        state.lastError = null;
      })
      .addCase(startAccessibilitySession.fulfilled, (state, action) => {
        state.isStartingSession = false;
        state.status = action.payload;
      })
      .addCase(startAccessibilitySession.rejected, (state, action) => {
        state.isStartingSession = false;
        state.lastError = (action.payload as string) ?? 'Failed to start accessibility session';
      })
      .addCase(stopAccessibilitySession.pending, state => {
        state.isStoppingSession = true;
        state.lastError = null;
      })
      .addCase(stopAccessibilitySession.fulfilled, (state, action) => {
        state.isStoppingSession = false;
        state.status = action.payload;
      })
      .addCase(stopAccessibilitySession.rejected, (state, action) => {
        state.isStoppingSession = false;
        state.lastError = (action.payload as string) ?? 'Failed to stop accessibility session';
      })
      .addCase(executeAccessibilityInputAction.rejected, (state, action) => {
        state.lastError = (action.payload as string) ?? 'Failed to execute accessibility action';
      });
  },
});

export const { clearAccessibilityError, setAccessibilitySessionFeatures, setAccessibilityStatus } =
  accessibilitySlice.actions;

export default accessibilitySlice.reducer;
