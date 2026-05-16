import { createSlice, type PayloadAction } from '@reduxjs/toolkit';
import { REHYDRATE } from 'redux-persist';

import type { MascotColor } from '../features/human/Mascot/mascotPalette';
import { resetUserScopedState } from './resetActions';

export const SUPPORTED_MASCOT_COLORS: readonly MascotColor[] = [
  'yellow',
  'burgundy',
  'black',
  'navy',
  'green',
];

export const DEFAULT_MASCOT_COLOR: MascotColor = 'yellow';

/**
 * Maximum length of a stored mascot voice id. ElevenLabs voice ids are
 * short opaque alphanumeric strings (typically 20 chars); the cap exists
 * solely so a stray paste of multi-megabyte clipboard data can never
 * land in localStorage and balloon the persisted blob. Anything longer
 * is dropped at the reducer boundary.
 */
export const MAX_MASCOT_VOICE_ID_LEN = 128;

/**
 * Loose shape check for a stored mascot voice id. Issue #1762 lets users
 * paste a custom ElevenLabs voice id, so we cannot enumerate the valid
 * set — instead we accept any non-empty trimmed string under the length
 * cap. The TTS path (`synthesizeSpeech` in
 * `app/src/features/human/voice/ttsClient.ts`) is the authoritative
 * gate: a syntactically valid id that ElevenLabs rejects falls back
 * cleanly via the existing TTS error handling, leaving `MASCOT_VOICE_ID`
 * as the implicit safe default.
 */
function isMascotVoiceId(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    value.trim().length > 0 &&
    value.trim().length <= MAX_MASCOT_VOICE_ID_LEN
  );
}

export interface MascotState {
  color: MascotColor;
  /**
   * User-selected ElevenLabs voice id for the mascot's reply speech, or
   * `null` to use the build-time default (`MASCOT_VOICE_ID` in
   * `app/src/utils/config.ts`). Issue #1762: surfaces what was
   * previously a build-time-only env var (`VITE_MASCOT_VOICE_ID`) as a
   * persisted user preference so the choice survives restarts and a
   * reset is just `setMascotVoiceId(null)`.
   */
  voiceId: string | null;
}

const initialState: MascotState = { color: DEFAULT_MASCOT_COLOR, voiceId: null };

function isMascotColor(value: unknown): value is MascotColor {
  return (
    typeof value === 'string' && (SUPPORTED_MASCOT_COLORS as readonly string[]).includes(value)
  );
}

const mascotSlice = createSlice({
  name: 'mascot',
  initialState,
  reducers: {
    setMascotColor(state, action: PayloadAction<MascotColor>) {
      if (isMascotColor(action.payload)) {
        state.color = action.payload;
      }
    },
    /**
     * Set or clear the user-selected mascot voice id. Whitespace is
     * trimmed; empty / oversize / non-string values clear the override
     * (falling back to the build-time default voice). Pass `null` from
     * the UI's Reset button to explicitly drop the override.
     */
    setMascotVoiceId(state, action: PayloadAction<string | null>) {
      if (action.payload == null) {
        state.voiceId = null;
        return;
      }
      if (isMascotVoiceId(action.payload)) {
        state.voiceId = action.payload.trim();
      } else {
        // Invalid input is treated as a reset rather than left in place
        // — a half-typed or junk-pasted value would otherwise silently
        // poison the TTS path on the next reply.
        state.voiceId = null;
      }
    },
  },
  extraReducers: builder => {
    builder.addCase(resetUserScopedState, () => initialState);
    // Guard against unknown/missing values surviving a rehydrate (e.g.
    // a future build removed a variant that was previously persisted).
    builder.addCase(REHYDRATE, (state, action) => {
      const rehydrateAction = action as {
        type: typeof REHYDRATE;
        key: string;
        payload?: { color?: unknown; voiceId?: unknown };
      };
      if (rehydrateAction.key !== 'mascot') return;
      const restoredColor = rehydrateAction.payload?.color;
      state.color = isMascotColor(restoredColor) ? restoredColor : DEFAULT_MASCOT_COLOR;
      // `voiceId` is optional in older persisted blobs (pre-#1762) — the
      // `null` fallback is the intended default and matches a fresh
      // install. Invalid values are scrubbed so a corrupted localStorage
      // blob can never make it into the TTS payload.
      const restoredVoiceId = rehydrateAction.payload?.voiceId;
      state.voiceId =
        restoredVoiceId == null
          ? null
          : isMascotVoiceId(restoredVoiceId)
            ? (restoredVoiceId as string).trim()
            : null;
    });
  },
});

export const { setMascotColor, setMascotVoiceId } = mascotSlice.actions;

export const selectMascotColor = (state: { mascot: MascotState }): MascotColor =>
  state.mascot.color;

export const selectMascotVoiceId = (state: { mascot: MascotState }): string | null =>
  state.mascot.voiceId;

export { mascotSlice };
export default mascotSlice.reducer;
