/**
 * Curated ElevenLabs voice ids exposed in the Mascot Voice picker
 * (issue #1762). Picked from the public ElevenLabs voice library so
 * users can swap to a different tone (incl. female voices) without
 * pasting an opaque id by hand.
 *
 * Adding a voice: keep the list short (≤ 12) so the dropdown fits a
 * single scroll-free view. Anything beyond this curated set is still
 * reachable via the "Other…" paste input — that's the escape hatch for
 * voices the user has cloned in their own ElevenLabs account.
 *
 * Ids match the ones documented at https://api.elevenlabs.io/v1/voices
 * (public library) — they are stable across ElevenLabs API versions, so
 * we hard-code rather than fetch at runtime (an extra round trip per
 * panel mount, plus offline-first considerations, both argue against a
 * runtime fetch for what is effectively a static menu).
 *
 * All presets render through the `eleven_multilingual_v2` model on the
 * backend, so each voice can speak any of the locales we ship — the
 * `locales` field below is a "natively fluent" hint used to pick a
 * default when the user opts into locale-based voice selection.
 */
import type { Locale } from '../../../lib/i18n/types';

export interface ElevenLabsVoicePreset {
  /** ElevenLabs voice id — opaque alphanumeric, sent verbatim to the TTS RPC. */
  id: string;
  /** Display label rendered in the dropdown. Includes accent + gender hints. */
  label: string;
  /** Coarse gender bucket for filtering / a11y descriptions. */
  gender: 'male' | 'female';
  /**
   * Locales this voice sounds natural in. Used only to power the
   * "auto-pick voice from app locale" toggle; the voice still works for
   * other locales through `eleven_multilingual_v2`, it just may not
   * carry a native accent. `'*'` means "good fallback for any locale".
   */
  locales: readonly (Locale | '*')[];
}

export const ELEVENLABS_VOICE_PRESETS: readonly ElevenLabsVoicePreset[] = [
  // Default mascot voice — keep first so the picker always offers a
  // path back to the shipped behaviour even when the env override is
  // unset. Matches `MASCOT_VOICE_ID` in `app/src/utils/config.ts`.
  // George (multilingual). `locales: ['*']` keeps it visible in both
  // the female- and male-filtered dropdowns so it's always one click
  // away as a "safe fallback".
  {
    id: 'JBFqnCBsd6RMkjVDRZzb',
    label: 'George · multilingual (male)',
    gender: 'male',
    locales: ['*'],
  },
  // Public ElevenLabs library voices — stable ids, mix of accents.
  { id: '21m00Tcm4TlvDq8ikWAM', label: 'Rachel · US (female)', gender: 'female', locales: ['en'] },
  { id: 'EXAVITQu4vr4xnSDxMaL', label: 'Bella · US (female)', gender: 'female', locales: ['en'] },
  { id: 'AZnzlk1XvdvUeBnXmlld', label: 'Domi · US (female)', gender: 'female', locales: ['en'] },
  {
    id: 'MF3mGyEYCl7XYWbV9V6O',
    label: 'Elli · US (female, young)',
    gender: 'female',
    locales: ['en'],
  },
  {
    id: 'jsCqWAovK2LkecY7zXl4',
    label: 'Freya · US (female, expressive)',
    gender: 'female',
    locales: ['en'],
  },
  { id: 'pNInz6obpgDQGcFmaJgB', label: 'Adam · US (male)', gender: 'male', locales: ['en'] },
  { id: 'ErXwobaYiN019PkySvjV', label: 'Antoni · US (male)', gender: 'male', locales: ['en'] },
  {
    id: 'VR6AewLTigWG4xSOukaG',
    label: 'Arnold · US (male, mature)',
    gender: 'male',
    locales: ['en'],
  },
  { id: 'TxGEqnHWrfWFTfGW9XjX', label: 'Josh · US (male, deep)', gender: 'male', locales: ['en'] },
];

/**
 * Per-locale default voice id, keyed by gender. Used by the "default
 * voice from app locale" toggle in the Mascot settings panel — when
 * enabled, the mascot speaks with this voice regardless of any manual
 * `voiceId` override.
 *
 * Every voice in the curated preset list renders through ElevenLabs'
 * `eleven_multilingual_v2` model, so the same id works in any locale;
 * we still curate per-locale picks here so a future expansion of the
 * preset list (e.g. adding a French-native voice) only needs to flip
 * the entry below, not every call site.
 *
 * `en` covers the default. Other locales fall back to it via
 * `defaultVoiceIdForLocale()` when a specific entry is missing.
 */
export const DEFAULT_VOICE_BY_LOCALE: Readonly<
  Partial<Record<Locale, Readonly<Record<'female' | 'male', string>>>>
> = {
  // Female default: Rachel — neutral, widely-used. Male default: Adam.
  en: { female: '21m00Tcm4TlvDq8ikWAM', male: 'pNInz6obpgDQGcFmaJgB' },
  'zh-CN': { female: 'EXAVITQu4vr4xnSDxMaL', male: 'pNInz6obpgDQGcFmaJgB' },
  hi: { female: 'EXAVITQu4vr4xnSDxMaL', male: 'ErXwobaYiN019PkySvjV' },
  es: { female: 'jsCqWAovK2LkecY7zXl4', male: 'ErXwobaYiN019PkySvjV' },
  ar: { female: 'AZnzlk1XvdvUeBnXmlld', male: 'VR6AewLTigWG4xSOukaG' },
  fr: { female: 'jsCqWAovK2LkecY7zXl4', male: 'ErXwobaYiN019PkySvjV' },
  bn: { female: 'EXAVITQu4vr4xnSDxMaL', male: 'ErXwobaYiN019PkySvjV' },
  pt: { female: 'AZnzlk1XvdvUeBnXmlld', male: 'ErXwobaYiN019PkySvjV' },
  ru: { female: 'AZnzlk1XvdvUeBnXmlld', male: 'VR6AewLTigWG4xSOukaG' },
  id: { female: 'EXAVITQu4vr4xnSDxMaL', male: 'pNInz6obpgDQGcFmaJgB' },
  it: { female: 'jsCqWAovK2LkecY7zXl4', male: 'ErXwobaYiN019PkySvjV' },
};

/**
 * Resolve the locale-default voice id for a given gender. Falls back to
 * English when a locale has no explicit entry — every entry in the
 * preset list works through `eleven_multilingual_v2`, so the English
 * default still produces correct (if accented) audio.
 */
export function defaultVoiceIdForLocale(locale: Locale, gender: 'male' | 'female'): string {
  const entry = DEFAULT_VOICE_BY_LOCALE[locale] ?? DEFAULT_VOICE_BY_LOCALE.en;
  return entry![gender];
}

/**
 * True iff `id` matches one of the curated presets above. Used by the
 * panel to decide whether to render the dropdown selection or fall
 * through to the "Other…" custom-paste editor — a custom id picked
 * outside the preset set should keep the paste editor open so the user
 * can see exactly what is stored.
 */
export function isCuratedVoicePreset(id: string | null | undefined): boolean {
  if (!id) return false;
  return ELEVENLABS_VOICE_PRESETS.some(p => p.id === id);
}
