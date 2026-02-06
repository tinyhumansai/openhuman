import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

export interface ModelStatus {
  available: boolean;
  loaded: boolean;
  loading: boolean;
  downloaded: boolean;
  downloadProgress: number | null;
  error: string | null;
  modelPath: string | null;
}

interface ModelState extends ModelStatus {
  /** Whether auto-download has been triggered this session */
  downloadTriggered: boolean;
}

const initialState: ModelState = {
  available: false,
  loaded: false,
  loading: false,
  downloaded: false,
  downloadProgress: null,
  error: null,
  modelPath: null,
  downloadTriggered: false,
};

const modelSlice = createSlice({
  name: 'model',
  initialState,
  reducers: {
    setModelStatus(state, action: PayloadAction<ModelStatus>) {
      const s = action.payload;
      state.available = s.available;
      state.loaded = s.loaded;
      state.loading = s.loading;
      state.downloaded = s.downloaded;
      state.downloadProgress = s.downloadProgress;
      state.error = s.error;
      state.modelPath = s.modelPath;
    },
    setDownloadTriggered(state, action: PayloadAction<boolean>) {
      state.downloadTriggered = action.payload;
    },
    setModelLoading(state, action: PayloadAction<boolean>) {
      state.loading = action.payload;
    },
    setModelError(state, action: PayloadAction<string | null>) {
      state.error = action.payload;
      if (action.payload) {
        state.loading = false;
      }
    },
  },
});

export const { setModelStatus, setDownloadTriggered, setModelLoading, setModelError } =
  modelSlice.actions;
export default modelSlice.reducer;
