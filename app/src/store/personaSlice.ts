import { createSlice, type PayloadAction } from '@reduxjs/toolkit';
import { REHYDRATE } from 'redux-persist';

import { resetUserScopedState } from './resetActions';

/**
 * Persona Pack v1 (issue #2345) — lightweight identity metadata the user sets
 * for their assistant. The actual personality lives in the editable `SOUL.md`
 * prompt (handled over RPC in the Persona settings panel); this slice only
 * holds the cosmetic display name + description, persisted locally the same way
 * mascot preferences are.
 */

/**
 * Caps on the persisted persona strings. These are display fields, not prompts,
 * so the limits are generous-but-bounded purely to stop a stray multi-megabyte
 * paste from landing in localStorage. Oversize input is rejected at the reducer
 * boundary rather than truncated, so the user keeps editing their draft.
 */
export const MAX_PERSONA_DISPLAY_NAME_LEN = 80;
export const MAX_PERSONA_DESCRIPTION_LEN = 500;

export interface PersonaState {
  /** User-facing name for the assistant persona, or `''` when unset. */
  displayName: string;
  /** Short free-text description of the persona, or `''` when unset. */
  description: string;
}

const initialState: PersonaState = { displayName: '', description: '' };

/**
 * Normalize a persona string: coerce non-strings to `''`, trim, and reject
 * (return `null`) when it exceeds `maxLen` so the caller can leave the prior
 * value in place. An empty/whitespace-only string is a valid "cleared" value.
 */
function normalizePersonaField(value: unknown, maxLen: number): string | null {
  if (typeof value !== 'string') return '';
  const trimmed = value.trim();
  if (trimmed.length > maxLen) return null;
  return trimmed;
}

const personaSlice = createSlice({
  name: 'persona',
  initialState,
  reducers: {
    setPersonaDisplayName(state, action: PayloadAction<string>) {
      const next = normalizePersonaField(action.payload, MAX_PERSONA_DISPLAY_NAME_LEN);
      if (next !== null) state.displayName = next;
    },
    setPersonaDescription(state, action: PayloadAction<string>) {
      const next = normalizePersonaField(action.payload, MAX_PERSONA_DESCRIPTION_LEN);
      if (next !== null) state.description = next;
    },
    /** Clear both persona fields back to their unset defaults. */
    resetPersona(state) {
      state.displayName = '';
      state.description = '';
    },
  },
  extraReducers: builder => {
    builder.addCase(resetUserScopedState, () => initialState);
    // Scrub anything unexpected (corrupted blob, a future build that dropped a
    // field) on rehydrate so the persisted value can never poison the UI.
    builder.addCase(REHYDRATE, (state, action) => {
      const rehydrateAction = action as {
        type: typeof REHYDRATE;
        key: string;
        payload?: { displayName?: unknown; description?: unknown };
      };
      if (rehydrateAction.key !== 'persona') return;
      state.displayName =
        normalizePersonaField(rehydrateAction.payload?.displayName, MAX_PERSONA_DISPLAY_NAME_LEN) ??
        '';
      state.description =
        normalizePersonaField(rehydrateAction.payload?.description, MAX_PERSONA_DESCRIPTION_LEN) ??
        '';
    });
  },
});

export const { setPersonaDisplayName, setPersonaDescription, resetPersona } = personaSlice.actions;

export const selectPersonaDisplayName = (state: { persona: PersonaState }): string =>
  state.persona.displayName;

export const selectPersonaDescription = (state: { persona: PersonaState }): string =>
  state.persona.description;

export { personaSlice };
export default personaSlice.reducer;
