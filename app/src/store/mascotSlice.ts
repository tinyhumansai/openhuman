import { createSlice, type PayloadAction } from '@reduxjs/toolkit';
import { REHYDRATE } from 'redux-persist';

import {
  defaultVoiceIdForLocale,
  ELEVENLABS_VOICE_PRESETS,
} from '../components/settings/panels/elevenlabsVoicePresets';
import type { MascotColor } from '../features/human/Mascot/mascotPalette';
import type { Locale } from '../lib/i18n/types';
import { MASCOT_VOICE_ID } from '../utils/config';
import { resetUserScopedState } from './resetActions';

export const SUPPORTED_MASCOT_COLORS: readonly MascotColor[] = [
  'yellow',
  'burgundy',
  'black',
  'navy',
  'custom',
];

export const DEFAULT_MASCOT_COLOR: MascotColor = 'yellow';

export type MascotVoiceGender = 'male' | 'female';

/**
 * Default gender for the mascot's reply voice. Matches the default
 * voice id (`MASCOT_VOICE_ID` — George, a male multilingual ElevenLabs
 * voice) so new users see consistent state in the Mascot settings
 * panel without any extra writes.
 */
export const DEFAULT_MASCOT_VOICE_GENDER: MascotVoiceGender = 'male';

/**
 * Maximum length of a stored mascot voice id. ElevenLabs voice ids are
 * short opaque alphanumeric strings (typically 20 chars); the cap exists
 * solely so a stray paste of multi-megabyte clipboard data can never
 * land in localStorage and balloon the persisted blob. Anything longer
 * is dropped at the reducer boundary.
 */
export const MAX_MASCOT_VOICE_ID_LEN = 128;
export const MAX_CUSTOM_MASCOT_GIF_URL_LEN = 2048;

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

function hasGifPath(value: string): boolean {
  const [path = ''] = value.split(/[?#]/, 1);
  return path.toLowerCase().endsWith('.gif');
}

export function isCustomMascotGifUrl(value: unknown): value is string {
  if (typeof value !== 'string') return false;
  const trimmed = value.trim();
  if (trimmed.length === 0 || trimmed.length > MAX_CUSTOM_MASCOT_GIF_URL_LEN) return false;

  try {
    const parsed = new URL(trimmed);
    if (!hasGifPath(parsed.pathname)) return false;
    if (parsed.protocol === 'https:' || parsed.protocol === 'file:') return true;
    if (parsed.protocol !== 'http:') return false;
    return ['localhost', '127.0.0.1', '::1', '[::1]'].includes(parsed.hostname);
  } catch {
    return hasGifPath(trimmed) && (trimmed.startsWith('/') || trimmed.startsWith('~/'));
  }
}

function isMascotVoiceGender(value: unknown): value is MascotVoiceGender {
  return value === 'male' || value === 'female';
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
  /**
   * Coarse gender bucket used by the Mascot settings panel to filter
   * the voice preset dropdown and to drive the "default voice from app
   * locale" toggle (combined with the current locale to pick a single
   * voice id). Independent of `voiceId` — the user can keep a manual
   * override and still flip gender for the locale-default branch.
   */
  voiceGender: MascotVoiceGender;
  /**
   * When true, ignore `voiceId` and pick the voice from the active
   * locale (+ `voiceGender`) via `defaultVoiceIdForLocale`. Lets users
   * say "speak in my UI language" once and have the mascot follow
   * locale changes without re-opening settings.
   */
  voiceUseLocaleDefault: boolean;
  /**
   * Server-side mascot id selected from the backend mascot library
   * (PR tinyhumansai/backend#770). `null` keeps the local YellowMascot
   * renderer; any non-empty value tells `BackendMascot` (loaded via
   * `mascotService`) to take over. The id is opaque server-side and
   * length-capped at the same threshold as voiceId to keep the
   * persisted blob bounded.
   */
  selectedMascotId: string | null;
  /**
   * User-supplied animated avatar source. Kept as a plain validated
   * string so the renderer can fall back to YellowMascot whenever the
   * override is absent or scrubbed during rehydrate.
   */
  customMascotGifUrl: string | null;
  customPrimaryColor: string;
  customSecondaryColor: string;
}

const initialState: MascotState = {
  color: DEFAULT_MASCOT_COLOR,
  voiceId: null,
  voiceGender: DEFAULT_MASCOT_VOICE_GENDER,
  voiceUseLocaleDefault: false,
  selectedMascotId: null,
  customMascotGifUrl: null,
  customPrimaryColor: '#F7D145',
  customSecondaryColor: '#B23C05',
};

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
     * Select a backend mascot by id. Trimmed; empty / oversize / null
     * clears the override and falls back to the local YellowMascot.
     */
    setSelectedMascotId(state, action: PayloadAction<string | null>) {
      if (action.payload == null) {
        state.selectedMascotId = null;
        return;
      }
      if (isMascotVoiceId(action.payload)) {
        state.selectedMascotId = action.payload.trim();
        state.customMascotGifUrl = null;
      } else {
        state.selectedMascotId = null;
      }
    },
    setCustomMascotGifUrl(state, action: PayloadAction<string | null>) {
      if (action.payload == null) {
        state.customMascotGifUrl = null;
        return;
      }
      if (isCustomMascotGifUrl(action.payload)) {
        state.customMascotGifUrl = action.payload.trim();
        state.selectedMascotId = null;
      } else {
        state.customMascotGifUrl = null;
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
    setMascotVoiceGender(state, action: PayloadAction<MascotVoiceGender>) {
      if (isMascotVoiceGender(action.payload)) {
        state.voiceGender = action.payload;
      }
    },
    setMascotVoiceUseLocaleDefault(state, action: PayloadAction<boolean>) {
      state.voiceUseLocaleDefault = Boolean(action.payload);
    },
    setCustomPrimaryColor(state, action: PayloadAction<string>) {
      state.customPrimaryColor = action.payload;
    },
    setCustomSecondaryColor(state, action: PayloadAction<string>) {
      state.customSecondaryColor = action.payload;
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
        payload?: {
          color?: unknown;
          voiceId?: unknown;
          voiceGender?: unknown;
          voiceUseLocaleDefault?: unknown;
          selectedMascotId?: unknown;
          customMascotGifUrl?: unknown;
          customPrimaryColor?: unknown;
          customSecondaryColor?: unknown;
        };
      };
      if (rehydrateAction.key !== 'mascot') return;
      const restoredColor = rehydrateAction.payload?.color;
      state.color = isMascotColor(restoredColor) ? restoredColor : DEFAULT_MASCOT_COLOR;
      const restoredSelectedMascotId = rehydrateAction.payload?.selectedMascotId;
      state.selectedMascotId =
        restoredSelectedMascotId == null
          ? null
          : isMascotVoiceId(restoredSelectedMascotId)
            ? (restoredSelectedMascotId as string).trim()
            : null;
      const restoredCustomMascotGifUrl = rehydrateAction.payload?.customMascotGifUrl;
      state.customMascotGifUrl =
        restoredCustomMascotGifUrl == null
          ? null
          : isCustomMascotGifUrl(restoredCustomMascotGifUrl)
            ? (restoredCustomMascotGifUrl as string).trim()
            : null;
      if (state.customMascotGifUrl) state.selectedMascotId = null;
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
      const restoredGender = rehydrateAction.payload?.voiceGender;
      state.voiceGender = isMascotVoiceGender(restoredGender)
        ? restoredGender
        : DEFAULT_MASCOT_VOICE_GENDER;
      state.voiceUseLocaleDefault =
        typeof rehydrateAction.payload?.voiceUseLocaleDefault === 'boolean'
          ? rehydrateAction.payload.voiceUseLocaleDefault
          : false;
      const rpc = rehydrateAction.payload?.customPrimaryColor;
      state.customPrimaryColor =
        typeof rpc === 'string' && rpc.length > 0 ? rpc : initialState.customPrimaryColor;
      const rsc = rehydrateAction.payload?.customSecondaryColor;
      state.customSecondaryColor =
        typeof rsc === 'string' && rsc.length > 0 ? rsc : initialState.customSecondaryColor;
    });
  },
});

export const {
  setMascotColor,
  setMascotVoiceId,
  setMascotVoiceGender,
  setMascotVoiceUseLocaleDefault,
  setSelectedMascotId,
  setCustomMascotGifUrl,
  setCustomPrimaryColor,
  setCustomSecondaryColor,
} = mascotSlice.actions;

export const selectMascotColor = (state: { mascot: MascotState }): MascotColor =>
  state.mascot.color;

export const selectMascotVoiceId = (state: { mascot: MascotState }): string | null =>
  state.mascot.voiceId;

export const selectMascotVoiceGender = (state: { mascot: MascotState }): MascotVoiceGender =>
  state.mascot.voiceGender;

export const selectMascotVoiceUseLocaleDefault = (state: { mascot: MascotState }): boolean =>
  state.mascot.voiceUseLocaleDefault;

export const selectSelectedMascotId = (state: { mascot: MascotState }): string | null =>
  state.mascot.selectedMascotId;

export const selectCustomMascotGifUrl = (state: { mascot: MascotState }): string | null =>
  state.mascot.customMascotGifUrl;

export const selectCustomPrimaryColor = (state: { mascot: MascotState }): string =>
  state.mascot.customPrimaryColor;

export const selectCustomSecondaryColor = (state: { mascot: MascotState }): string =>
  state.mascot.customSecondaryColor;

/**
 * Resolve the voice id the next reply will be synthesised with, taking
 * into account every mascot-voice setting plus the active locale. This
 * is the single source of truth read by both UI ("what does the picker
 * show as current?") and the TTS hook ("what voice should I pass to
 * synthesizeSpeech?"), so they can never drift.
 *
 * Resolution order:
 *   1. `voiceUseLocaleDefault` on → locale-default for `voiceGender`.
 *   2. Manual `voiceId` set → that id.
 *   3. Otherwise → `MASCOT_VOICE_ID` (the build-time default).
 *
 * The first branch deliberately wins over a manual override so the
 * "speak in my UI language" toggle behaves predictably — flipping it on
 * without first clearing a stale override would otherwise silently do
 * nothing. The UI in `MascotPanel` makes this precedence visible by
 * disabling the manual picker while the toggle is on.
 */
export const selectEffectiveMascotVoiceId = (state: {
  mascot: MascotState;
  locale?: { current: Locale };
}): string => {
  if (state.mascot.voiceUseLocaleDefault) {
    // `locale` slice may be absent in narrow test harnesses (e.g.
    // MascotPanel.test wires only the mascot reducer). Default to `en`
    // so the resolver still produces a usable id rather than throwing.
    const current = state.locale?.current ?? 'en';
    return defaultVoiceIdForLocale(current, state.mascot.voiceGender);
  }
  if (state.mascot.voiceId) return state.mascot.voiceId;
  // Belt-and-braces: if the build-time default ever drops out of the
  // curated preset list, fall back to the first preset rather than a
  // bogus empty string.
  return MASCOT_VOICE_ID || ELEVENLABS_VOICE_PRESETS[0].id;
};

export { mascotSlice };
export default mascotSlice.reducer;
