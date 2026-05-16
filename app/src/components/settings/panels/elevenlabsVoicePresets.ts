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
 */
export interface ElevenLabsVoicePreset {
  /** ElevenLabs voice id — opaque alphanumeric, sent verbatim to the TTS RPC. */
  id: string;
  /** Display label rendered in the dropdown. Includes accent + gender hints. */
  label: string;
  /** Coarse gender bucket for filtering / a11y descriptions. */
  gender: 'male' | 'female';
}

export const ELEVENLABS_VOICE_PRESETS: readonly ElevenLabsVoicePreset[] = [
  // Default mascot voice — keep first so the picker always offers a
  // path back to the shipped behaviour even when the env override is
  // unset. Matches `MASCOT_VOICE_ID` in `app/src/utils/config.ts`.
  { id: 'ljX1ZrXuDIIRVcmiVSyR', label: 'Default mascot voice', gender: 'female' },
  // Public ElevenLabs library voices — stable ids, mix of accents.
  { id: '21m00Tcm4TlvDq8ikWAM', label: 'Rachel · US (female)', gender: 'female' },
  { id: 'EXAVITQu4vr4xnSDxMaL', label: 'Bella · US (female)', gender: 'female' },
  { id: 'AZnzlk1XvdvUeBnXmlld', label: 'Domi · US (female)', gender: 'female' },
  { id: 'MF3mGyEYCl7XYWbV9V6O', label: 'Elli · US (female, young)', gender: 'female' },
  { id: 'jsCqWAovK2LkecY7zXl4', label: 'Freya · US (female, expressive)', gender: 'female' },
  { id: 'pNInz6obpgDQGcFmaJgB', label: 'Adam · US (male)', gender: 'male' },
  { id: 'ErXwobaYiN019PkySvjV', label: 'Antoni · US (male)', gender: 'male' },
  { id: 'VR6AewLTigWG4xSOukaG', label: 'Arnold · US (male, mature)', gender: 'male' },
  { id: 'TxGEqnHWrfWFTfGW9XjX', label: 'Josh · US (male, deep)', gender: 'male' },
];

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
