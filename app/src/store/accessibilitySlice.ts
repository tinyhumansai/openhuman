import { createAsyncThunk, createSlice, type PayloadAction } from '@reduxjs/toolkit';

import {
  type AccessibilityInputActionParams,
  type AccessibilityPermissionKind,
  type AccessibilitySessionStatus,
  type AccessibilityStatus,
  type AccessibilityVisionSummary,
  type CaptureTestResult,
  openhumanAccessibilityInputAction,
  openhumanAccessibilityRequestPermission,
  openhumanAccessibilityRequestPermissions,
  openhumanAccessibilityStartSession,
  openhumanAccessibilityStatus,
  openhumanAccessibilityStopSession,
  openhumanAccessibilityVisionFlush,
  openhumanAccessibilityVisionRecent,
  openhumanScreenIntelligenceCaptureTest,
  restartCoreProcess,
} from '../utils/tauriCommands';

interface AccessibilityState {
  status: AccessibilityStatus | null;
  recentVisionSummaries: AccessibilityVisionSummary[];
  captureTestResult: CaptureTestResult | null;
  isCaptureTestRunning: boolean;
  isLoading: boolean;
  isRequestingPermissions: boolean;
  isRestartingCore: boolean;
  isStartingSession: boolean;
  isStoppingSession: boolean;
  isLoadingVision: boolean;
  isFlushingVision: boolean;
  lastError: string | null;
}

const initialState: AccessibilityState = {
  status: null,
  recentVisionSummaries: [],
  captureTestResult: null,
  isCaptureTestRunning: false,
  isLoading: false,
  isRequestingPermissions: false,
  isRestartingCore: false,
  isStartingSession: false,
  isStoppingSession: false,
  isLoadingVision: false,
  isFlushingVision: false,
  lastError: null,
};

const extractError = (error: unknown, fallback: string): string => {
  if (error instanceof Error && error.message.trim()) {
    return error.message;
  }
  if (typeof error === 'string' && error.trim()) {
    return error;
  }
  if (error && typeof error === 'object') {
    const msg = (error as { message?: unknown }).message;
    if (typeof msg === 'string' && msg.trim()) {
      return msg;
    }
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

export const requestAccessibilityPermission = createAsyncThunk(
  'accessibility/requestPermission',
  async (permission: AccessibilityPermissionKind, { rejectWithValue }) => {
    try {
      await openhumanAccessibilityRequestPermission(permission);
      const response = await openhumanAccessibilityStatus();
      return response.result;
    } catch (error) {
      return rejectWithValue(extractError(error, 'Failed to request accessibility permission'));
    }
  }
);

/**
 * Restart the core sidecar and re-fetch accessibility status.
 *
 * macOS caches permission state per-process. The running sidecar never sees a
 * newly granted permission until it exits and a fresh process starts. This thunk:
 *   1. Restarts the core sidecar via the `restart_core_process` Tauri command.
 *   2. Waits briefly, then re-fetches status with retries while the new sidecar binds.
 *   3. Updates Redux so the UI reflects the updated grants.
 *
 * @see https://github.com/tinyhumansai/openhuman/issues/133 — stale DENIED after granting in System Settings
 */
export const refreshPermissionsWithRestart = createAsyncThunk(
  'accessibility/refreshPermissionsWithRestart',
  async (_, { rejectWithValue }) => {
    try {
      console.debug('[accessibility] refreshPermissionsWithRestart: starting core restart');
      await restartCoreProcess();
      console.debug('[accessibility] refreshPermissionsWithRestart: waiting for sidecar ready');
      await new Promise<void>(resolve => setTimeout(resolve, 400));
      console.debug('[accessibility] refreshPermissionsWithRestart: fetching updated status');
      for (let attempt = 1; attempt <= 5; attempt++) {
        try {
          const response = await openhumanAccessibilityStatus();
          console.debug(
            '[accessibility] refreshPermissionsWithRestart: done — screen_recording=%s accessibility=%s input_monitoring=%s',
            response.result.permissions.screen_recording,
            response.result.permissions.accessibility,
            response.result.permissions.input_monitoring
          );
          return response.result;
        } catch (e) {
          if (attempt === 5) {
            throw e;
          }
          console.debug(
            '[accessibility] refreshPermissionsWithRestart: status fetch failed (attempt %s), retrying…',
            attempt
          );
          await new Promise<void>(resolve => setTimeout(resolve, 350 * attempt));
        }
      }
      throw new Error('Failed to fetch accessibility status after core restart');
    } catch (error) {
      const msg = extractError(error, 'Failed to restart core and refresh permissions');
      console.error('[accessibility] refreshPermissionsWithRestart: error —', msg, error);
      return rejectWithValue(msg);
    }
  }
);

export const startAccessibilitySession = createAsyncThunk(
  'accessibility/startSession',
  async (
    params: { consent: boolean; ttl_secs?: number; screen_monitoring?: boolean },
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

export const fetchAccessibilityVisionRecent = createAsyncThunk(
  'accessibility/fetchVisionRecent',
  async (limit: number | undefined, { rejectWithValue }) => {
    try {
      const response = await openhumanAccessibilityVisionRecent(limit);
      return response.result.summaries;
    } catch (error) {
      return rejectWithValue(extractError(error, 'Failed to fetch accessibility vision summaries'));
    }
  }
);

export const flushAccessibilityVision = createAsyncThunk(
  'accessibility/flushVision',
  async (_, { rejectWithValue }) => {
    try {
      const response = await openhumanAccessibilityVisionFlush();
      return response.result.summary;
    } catch (error) {
      return rejectWithValue(extractError(error, 'Failed to flush accessibility vision'));
    }
  }
);

export const runCaptureTest = createAsyncThunk(
  'accessibility/captureTest',
  async (_, { rejectWithValue }) => {
    try {
      const response = await openhumanScreenIntelligenceCaptureTest();
      return response.result;
    } catch (error) {
      return rejectWithValue(extractError(error, 'Failed to run capture test'));
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
    setAccessibilityVisionSummaries(state, action: PayloadAction<AccessibilityVisionSummary[]>) {
      state.recentVisionSummaries = action.payload;
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
      .addCase(requestAccessibilityPermission.pending, state => {
        state.isRequestingPermissions = true;
        state.lastError = null;
      })
      .addCase(requestAccessibilityPermission.fulfilled, (state, action) => {
        state.isRequestingPermissions = false;
        state.status = action.payload;
      })
      .addCase(requestAccessibilityPermission.rejected, (state, action) => {
        state.isRequestingPermissions = false;
        state.lastError =
          (action.payload as string) ?? 'Failed to request accessibility permission';
      })
      .addCase(refreshPermissionsWithRestart.pending, state => {
        state.isRestartingCore = true;
        state.lastError = null;
      })
      .addCase(refreshPermissionsWithRestart.fulfilled, (state, action) => {
        state.isRestartingCore = false;
        state.status = action.payload;
      })
      .addCase(refreshPermissionsWithRestart.rejected, (state, action) => {
        state.isRestartingCore = false;
        state.lastError =
          (action.payload as string) ?? 'Failed to restart core and refresh permissions';
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
      })
      .addCase(fetchAccessibilityVisionRecent.pending, state => {
        state.isLoadingVision = true;
      })
      .addCase(fetchAccessibilityVisionRecent.fulfilled, (state, action) => {
        state.isLoadingVision = false;
        state.recentVisionSummaries = action.payload;
      })
      .addCase(fetchAccessibilityVisionRecent.rejected, (state, action) => {
        state.isLoadingVision = false;
        state.lastError =
          (action.payload as string) ?? 'Failed to fetch accessibility vision summaries';
      })
      .addCase(flushAccessibilityVision.pending, state => {
        state.isFlushingVision = true;
      })
      .addCase(flushAccessibilityVision.fulfilled, (state, action) => {
        state.isFlushingVision = false;
        if (action.payload) {
          state.recentVisionSummaries = [action.payload, ...state.recentVisionSummaries].slice(
            0,
            30
          );
        }
      })
      .addCase(flushAccessibilityVision.rejected, (state, action) => {
        state.isFlushingVision = false;
        state.lastError = (action.payload as string) ?? 'Failed to flush accessibility vision';
      })
      .addCase(runCaptureTest.pending, state => {
        state.isCaptureTestRunning = true;
        state.captureTestResult = null;
        state.lastError = null;
      })
      .addCase(runCaptureTest.fulfilled, (state, action) => {
        state.isCaptureTestRunning = false;
        state.captureTestResult = action.payload;
      })
      .addCase(runCaptureTest.rejected, (state, action) => {
        state.isCaptureTestRunning = false;
        state.lastError = (action.payload as string) ?? 'Failed to run capture test';
      });
  },
});

export const {
  clearAccessibilityError,
  setAccessibilitySessionFeatures,
  setAccessibilityStatus,
  setAccessibilityVisionSummaries,
} = accessibilitySlice.actions;

export default accessibilitySlice.reducer;
