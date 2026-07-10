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
 * Upper bound on how many per-mascot voice overrides we persist (issue
 * #4277). A user only ever drives two mascots in a meeting, but they may
 * try several before settling; the cap keeps the persisted map bounded
 * against a runaway writer while comfortably covering real use. Once the
 * cap is reached the reducer refuses NEW keys (an existing mascot can
 * still be re-voiced); on rehydrate the first `MAX_MASCOT_VOICES` valid
 * entries are kept and the rest dropped.
 */
export const MAX_MASCOT_VOICES = 16;

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
   * Mascot id selected from the published GitHub manifest
   * (`tinyhumansai/mascots`, resolved via `useMascotManifest`). `null` falls
   * back to the manifest's default (first `ready`) mascot; any non-empty value
   * pins that specific mascot. The id is the manifest entry id (e.g.
   * `tiny-mascot`) and length-capped at the same threshold as voiceId to keep
   * the persisted blob bounded.
   */
  selectedMascotId: string | null;
  /**
   * Second mascot enabled for meetings (issue #4277). When set (and
   * distinct from `selectedMascotId`) the meeting bot shows both mascots
   * together and alternates who speaks each reply. `null` = single-mascot
   * behavior, unchanged. Same validation/length cap as `selectedMascotId`.
   */
  secondaryMascotId: string | null;
  /**
   * Per-mascot reply-voice overrides (issue #4277), keyed by manifest
   * mascot id → ElevenLabs voice id. Lets each mascot in a two-mascot
   * meeting speak in its own voice. Empty map = no per-mascot override,
   * so every mascot falls back to `selectEffectiveMascotVoiceId` (the
   * single-voice behavior). Bounded by `MAX_MASCOT_VOICES`.
   */
  mascotVoices: Record<string, string>;
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
  secondaryMascotId: null,
  mascotVoices: {},
  customMascotGifUrl: null,
  customPrimaryColor: '#F7D145',
  customSecondaryColor: '#B23C05',
};

/**
 * Scrub a persisted / raw `mascotVoices` blob down to valid
 * `mascotId → voiceId` entries under the size cap. Non-object inputs and
 * any entry whose key or value fails `isMascotVoiceId` are dropped, so a
 * corrupted localStorage blob can never poison the meeting TTS payload.
 */
function sanitizeMascotVoices(value: unknown): Record<string, string> {
  if (value == null || typeof value !== 'object' || Array.isArray(value)) return {};
  const out: Record<string, string> = {};
  for (const [key, val] of Object.entries(value as Record<string, unknown>)) {
    if (Object.keys(out).length >= MAX_MASCOT_VOICES) break;
    if (isMascotVoiceId(key) && isMascotVoiceId(val)) {
      out[key.trim()] = (val as string).trim();
    }
  }
  return out;
}

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
    /**
     * Enable / clear the second meeting mascot (issue #4277). Trimmed;
     * empty / oversize / null clears it (back to single-mascot). A custom
     * GIF avatar and a second Rive mascot are mutually exclusive, so
     * setting one clears the GIF override — mirroring `setSelectedMascotId`.
     */
    setSecondaryMascotId(state, action: PayloadAction<string | null>) {
      if (action.payload == null) {
        state.secondaryMascotId = null;
        return;
      }
      if (isMascotVoiceId(action.payload)) {
        state.secondaryMascotId = action.payload.trim();
        state.customMascotGifUrl = null;
      } else {
        state.secondaryMascotId = null;
      }
    },
    /**
     * Set or clear a per-mascot reply voice (issue #4277). A non-empty
     * valid `voiceId` records `mascotId → voiceId`; a `null`/invalid
     * `voiceId` removes the entry (that mascot falls back to the effective
     * single voice). Both key and value are validated + trimmed so junk
     * can't grow the persisted map. Over-cap writes are ignored.
     */
    setMascotVoice(state, action: PayloadAction<{ mascotId: string; voiceId: string | null }>) {
      const { mascotId, voiceId } = action.payload;
      if (!isMascotVoiceId(mascotId)) return;
      const key = mascotId.trim();
      if (voiceId == null || !isMascotVoiceId(voiceId)) {
        delete state.mascotVoices[key];
        return;
      }
      // Only enforce the cap when introducing a NEW key — updating an
      // existing mascot's voice must always be allowed.
      if (
        !(key in state.mascotVoices) &&
        Object.keys(state.mascotVoices).length >= MAX_MASCOT_VOICES
      ) {
        return;
      }
      state.mascotVoices[key] = voiceId.trim();
    },
    setCustomMascotGifUrl(state, action: PayloadAction<string | null>) {
      if (action.payload == null) {
        state.customMascotGifUrl = null;
        return;
      }
      if (isCustomMascotGifUrl(action.payload)) {
        state.customMascotGifUrl = action.payload.trim();
        state.selectedMascotId = null;
        state.secondaryMascotId = null;
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
          secondaryMascotId?: unknown;
          mascotVoices?: unknown;
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
      // Second mascot + per-mascot voices are absent in pre-#4277 blobs;
      // the `null` / `{}` fallbacks match a fresh install and keep
      // single-mascot users unchanged. Invalid values are scrubbed.
      const restoredSecondaryMascotId = rehydrateAction.payload?.secondaryMascotId;
      state.secondaryMascotId =
        restoredSecondaryMascotId == null
          ? null
          : isMascotVoiceId(restoredSecondaryMascotId)
            ? (restoredSecondaryMascotId as string).trim()
            : null;
      state.mascotVoices = sanitizeMascotVoices(rehydrateAction.payload?.mascotVoices);
      const restoredCustomMascotGifUrl = rehydrateAction.payload?.customMascotGifUrl;
      state.customMascotGifUrl =
        restoredCustomMascotGifUrl == null
          ? null
          : isCustomMascotGifUrl(restoredCustomMascotGifUrl)
            ? (restoredCustomMascotGifUrl as string).trim()
            : null;
      // A custom GIF avatar is mutually exclusive with Rive mascots —
      // drop both mascot selections if a GIF override survived.
      if (state.customMascotGifUrl) {
        state.selectedMascotId = null;
        state.secondaryMascotId = null;
      }
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
  setSecondaryMascotId,
  setMascotVoice,
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

export const selectSecondaryMascotId = (state: { mascot: MascotState }): string | null =>
  state.mascot.secondaryMascotId;

export const selectMascotVoices = (state: { mascot: MascotState }): Record<string, string> =>
  state.mascot.mascotVoices ?? {};

/**
 * Explicit per-mascot voice override for `mascotId`, or `null` when none
 * is set (caller falls back to the effective single voice). Curried so it
 * reads like the other parameterised selectors at call sites.
 */
export const selectMascotVoiceFor =
  (mascotId: string | null) =>
  (state: { mascot: MascotState }): string | null =>
    mascotId ? (state.mascot.mascotVoices?.[mascotId] ?? null) : null;

/**
 * True when a distinct second mascot is enabled — the single gate the
 * meeting render + join paths use to decide dual vs single behavior.
 * Guards against the same mascot being picked twice.
 */
export const selectDualMascotEnabled = (state: { mascot: MascotState }): boolean => {
  const { selectedMascotId, secondaryMascotId } = state.mascot;
  return secondaryMascotId != null && secondaryMascotId !== selectedMascotId;
};

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

export interface MeetingMascotSlot {
  /** Manifest mascot id, or `null` for the primary when the user is on
   *  the default (first-`ready`) mascot. */
  mascotId: string | null;
  /** Resolved voice id: the per-mascot override, else the effective
   *  single voice — never empty, so the join payload always carries one. */
  voiceId: string;
}

export interface MeetingMascotVoicePair {
  primary: MeetingMascotSlot;
  secondary: MeetingMascotSlot | null;
}

/**
 * Resolve the (up to two) mascots + voices a meeting join should use
 * (issue #4277). Single source of truth for both join paths — the CEF
 * `meet_call_open_window` sender and the backend `agent_meetings_join`
 * sender — and for tests, so they can't drift.
 *
 * `secondary` is non-null only when a distinct second mascot is enabled
 * (`selectDualMascotEnabled`). Each slot's voice is its per-mascot
 * override, falling back to `selectEffectiveMascotVoiceId`; when the user
 * hasn't set distinct voices both slots resolve to that same voice
 * (harmless — alternation still works, it just sounds the same).
 */
export const selectMeetingMascotVoicePair = (state: {
  mascot: MascotState;
  locale?: { current: Locale };
}): MeetingMascotVoicePair => {
  const effective = selectEffectiveMascotVoiceId(state);
  const { selectedMascotId, secondaryMascotId } = state.mascot;
  // Tolerate a partial / pre-migration mascot slice (e.g. a legacy persisted
  // blob or a test's preloadedState) that predates `mascotVoices`.
  const mascotVoices = state.mascot.mascotVoices ?? {};
  const primary: MeetingMascotSlot = {
    mascotId: selectedMascotId,
    voiceId: (selectedMascotId && mascotVoices[selectedMascotId]) || effective,
  };
  const dualEnabled = secondaryMascotId != null && secondaryMascotId !== selectedMascotId;
  const secondary: MeetingMascotSlot | null = dualEnabled
    ? { mascotId: secondaryMascotId, voiceId: mascotVoices[secondaryMascotId] || effective }
    : null;
  return { primary, secondary };
};

export { mascotSlice };
export default mascotSlice.reducer;
