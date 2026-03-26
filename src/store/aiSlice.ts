import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

/** AI system connection/initialization status */
export type AIStatus = 'idle' | 'initializing' | 'ready' | 'error';

/** Persisted AI configuration */
export interface AIConfig {
  /** Custom LLM endpoint URL */
  llmEndpoint?: string;
  /** Custom LLM model name */
  llmModel?: string;
  /** Embedding provider: 'openai' | 'custom' | 'none' */
  embeddingProvider?: string;
  /** OpenAI API key (for embeddings fallback) */
  openaiApiKey?: string;
  /** Web search API endpoint */
  webSearchEndpoint?: string;
  /** Web search API key */
  webSearchApiKey?: string;
  /** Skills repo URL */
  skillsRepoUrl?: string;
}

interface AIState {
  /** Current AI system status */
  status: AIStatus;
  /** Error message if status is 'error' */
  error: string | null;
  /** Current active session ID */
  currentSessionId: string | null;
  /** Number of loaded skills */
  loadedSkillsCount: number;
  /** Memory system initialized */
  memoryInitialized: boolean;
  /** Persisted AI configuration */
  config: AIConfig;
}

const initialState: AIState = {
  status: 'idle',
  error: null,
  currentSessionId: null,
  loadedSkillsCount: 0,
  memoryInitialized: false,
  config: { skillsRepoUrl: 'openhuman/openhuman-skills' },
};

const aiSlice = createSlice({
  name: 'ai',
  initialState,
  reducers: {
    setAIStatus(state, action: PayloadAction<AIStatus>) {
      state.status = action.payload;
      if (action.payload !== 'error') {
        state.error = null;
      }
    },
    setAIError(state, action: PayloadAction<string>) {
      state.status = 'error';
      state.error = action.payload;
    },
    setCurrentSessionId(state, action: PayloadAction<string | null>) {
      state.currentSessionId = action.payload;
    },
    setLoadedSkillsCount(state, action: PayloadAction<number>) {
      state.loadedSkillsCount = action.payload;
    },
    setMemoryInitialized(state, action: PayloadAction<boolean>) {
      state.memoryInitialized = action.payload;
    },
    updateAIConfig(state, action: PayloadAction<Partial<AIConfig>>) {
      state.config = { ...state.config, ...action.payload };
    },
    resetAIState() {
      return initialState;
    },
  },
});

export const {
  setAIStatus,
  setAIError,
  setCurrentSessionId,
  setLoadedSkillsCount,
  setMemoryInitialized,
  updateAIConfig,
  resetAIState,
} = aiSlice.actions;

export default aiSlice.reducer;
