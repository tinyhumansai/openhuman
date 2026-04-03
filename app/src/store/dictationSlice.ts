import { createAsyncThunk, createSlice, type PayloadAction } from '@reduxjs/toolkit';

import { callCoreRpc } from '../services/coreRpcClient';

export type DictationStatus = 'idle' | 'recording' | 'transcribing' | 'ready' | 'error';

export interface VoiceStatusResult {
  stt_available: boolean;
  tts_available: boolean;
  stt_model_id: string;
  tts_voice_id: string;
  whisper_binary: string | null;
  piper_binary: string | null;
  stt_model_path: string | null;
  tts_voice_path: string | null;
  whisper_in_process: boolean;
  llm_cleanup_enabled: boolean;
}

interface DictationState {
  status: DictationStatus;
  transcript: string | null;
  error: string | null;
  hotkey: string;
  showFloatingLauncher: boolean;
  sttAvailable: boolean;
  voiceStatus: VoiceStatusResult | null;
  statusCheckError: string | null;
  isCheckingStatus: boolean;
}

const DEFAULT_HOTKEY = 'CommandOrControl+Shift+D';

const initialState: DictationState = {
  status: 'idle',
  transcript: null,
  error: null,
  hotkey: DEFAULT_HOTKEY,
  showFloatingLauncher: true,
  sttAvailable: false,
  voiceStatus: null,
  statusCheckError: null,
  isCheckingStatus: false,
};

// voice/schemas.rs to_json() serializes outcome.value directly (no {result,logs} wrapper),
// so callCoreRpc returns the VoiceStatus object itself.
export const checkDictationAvailability = createAsyncThunk(
  'dictation/checkAvailability',
  async (_, { rejectWithValue }) => {
    try {
      const status = await callCoreRpc<VoiceStatusResult>({
        method: 'openhuman.voice_status',
        params: {},
      });
      console.debug(
        '[dictation] voice_status: stt_available=%s whisper_in_process=%s model_path=%s binary=%s',
        status.stt_available,
        status.whisper_in_process,
        status.stt_model_path,
        status.whisper_binary
      );
      return status;
    } catch (error) {
      const msg = error instanceof Error ? error.message : 'Failed to check voice status';
      console.error('[dictation] voice_status check failed:', msg);
      return rejectWithValue(msg);
    }
  }
);

const dictationSlice = createSlice({
  name: 'dictation',
  initialState,
  reducers: {
    setStatus(state, action: PayloadAction<DictationStatus>) {
      state.status = action.payload;
    },
    setTranscript(state, action: PayloadAction<string | null>) {
      state.transcript = action.payload;
      if (action.payload !== null) {
        state.status = 'ready';
      }
    },
    setError(state, action: PayloadAction<string | null>) {
      state.error = action.payload;
      if (action.payload !== null) {
        state.status = 'error';
      }
    },
    setHotkey(state, action: PayloadAction<string>) {
      state.hotkey = action.payload;
    },
    setShowFloatingLauncher(state, action: PayloadAction<boolean>) {
      state.showFloatingLauncher = action.payload;
    },
    reset(state) {
      state.status = 'idle';
      state.transcript = null;
      state.error = null;
    },
  },
  extraReducers: builder => {
    builder
      .addCase(checkDictationAvailability.pending, state => {
        state.isCheckingStatus = true;
        state.statusCheckError = null;
      })
      .addCase(checkDictationAvailability.fulfilled, (state, action) => {
        state.isCheckingStatus = false;
        state.voiceStatus = action.payload;
        // Consider STT available only when the model file exists AND there is a way to run
        // inference: either the in-process engine is already loaded, or a whisper binary is
        // present for the subprocess fallback.  Showing the overlay ready when only the model
        // file exists but no binary/engine is available would let the user attempt recording
        // that then fails at transcription time with a confusing error.
        state.sttAvailable =
          action.payload.stt_available ||
          (action.payload.stt_model_path !== null &&
            (action.payload.whisper_in_process || action.payload.whisper_binary !== null));
      })
      .addCase(checkDictationAvailability.rejected, (state, action) => {
        state.isCheckingStatus = false;
        state.sttAvailable = false;
        state.statusCheckError = (action.payload as string) ?? 'Unknown error';
      });
  },
});

export const {
  setStatus,
  setTranscript,
  setError,
  setHotkey,
  setShowFloatingLauncher,
  reset: resetDictation,
} = dictationSlice.actions;
export default dictationSlice.reducer;
