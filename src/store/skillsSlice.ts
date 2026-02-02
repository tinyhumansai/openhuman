import { createSlice, type PayloadAction } from '@reduxjs/toolkit';

import type {
  SkillManifest,
  SkillState,
  SkillStatus,
  SkillToolDefinition,
} from '../lib/skills/types';

interface SkillsState {
  /** Skill states keyed by skill ID */
  skills: Record<string, SkillState>;
  /** Arbitrary per-skill state for reverse RPC state/get and state/set */
  skillStates: Record<string, Record<string, unknown>>;
}

const initialState: SkillsState = { skills: {}, skillStates: {} };

const skillsSlice = createSlice({
  name: 'skills',
  initialState,
  reducers: {
    addSkill(state, action: PayloadAction<{ manifest: SkillManifest }>) {
      const { manifest } = action.payload;
      // Preserve existing setupComplete if re-adding
      const existing = state.skills[manifest.id];
      state.skills[manifest.id] = {
        manifest,
        status: 'installed',
        setupComplete: existing?.setupComplete ?? false,
        tools: existing?.tools ?? [],
      };
    },

    setSkillStatus(state, action: PayloadAction<{ skillId: string; status: SkillStatus }>) {
      const { skillId, status } = action.payload;
      if (state.skills[skillId]) {
        state.skills[skillId].status = status;
        if (status !== 'error') {
          delete state.skills[skillId].error;
        }
      }
    },

    setSkillError(state, action: PayloadAction<{ skillId: string; error: string }>) {
      const { skillId, error } = action.payload;
      if (state.skills[skillId]) {
        state.skills[skillId].status = 'error';
        state.skills[skillId].error = error;
      }
    },

    setSkillSetupComplete(state, action: PayloadAction<{ skillId: string; complete: boolean }>) {
      const { skillId, complete } = action.payload;
      if (state.skills[skillId]) {
        state.skills[skillId].setupComplete = complete;
      }
    },

    setSkillTools(state, action: PayloadAction<{ skillId: string; tools: SkillToolDefinition[] }>) {
      const { skillId, tools } = action.payload;
      if (state.skills[skillId]) {
        state.skills[skillId].tools = tools;
      }
    },

    setSkillState(
      state,
      action: PayloadAction<{ skillId: string; state: Record<string, unknown> }>
    ) {
      state.skillStates[action.payload.skillId] = action.payload.state;
    },

    removeSkill(state, action: PayloadAction<string>) {
      delete state.skills[action.payload];
      delete state.skillStates[action.payload];
    },

    resetSkillsState() {
      return initialState;
    },
  },
});

export const {
  addSkill,
  setSkillStatus,
  setSkillError,
  setSkillSetupComplete,
  setSkillTools,
  setSkillState,
  removeSkill,
  resetSkillsState,
} = skillsSlice.actions;

export default skillsSlice.reducer;
