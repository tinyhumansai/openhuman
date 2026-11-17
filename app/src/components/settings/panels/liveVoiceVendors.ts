import type { LiveVoiceProvider, LiveVoiceSettings } from '../../../services/api/liveVoiceApi';

/**
 * A voice service as the user sees it (Gemini Live, ElevenLabs, Sarvam), with
 * every way the core offers to reach it: managed by TinyHumans, the user's
 * own key, or both. The core lists those as separate providers.
 */
export interface LiveVoiceVendor {
  id: string;
  name: string;
  providers: LiveVoiceProvider[];
}

/** Description key per known vendor (explicit so the i18n audit sees them). */
const VENDOR_DESC_KEY: Record<string, string> = {
  gemini: 'connections.voiceAgents.vendorGemini',
  elevenlabs: 'connections.voiceAgents.vendorElevenlabs',
  sarvam: 'connections.voiceAgents.vendorSarvam',
};

export const vendorDescKey = (vendorId: string): string | undefined => VENDOR_DESC_KEY[vendorId];

/** `gemini-hosted` and `gemini` are the same service reached two ways. */
export const vendorIdOf = (providerId: string) => providerId.replace(/-hosted$/, '');

/** "Gemini Live (TinyHumans)" → "Gemini Live". */
const vendorName = (label: string) => label.replace(/\s*\([^)]*\)\s*$/, '') || label;

/** Group the core's providers into vendors, hosted variants first, in catalog order. */
export function groupVendors(providers: LiveVoiceProvider[]): LiveVoiceVendor[] {
  const vendors: LiveVoiceVendor[] = [];
  for (const provider of providers) {
    const id = vendorIdOf(provider.id);
    let vendor = vendors.find(v => v.id === id);
    if (!vendor) {
      vendor = { id, name: vendorName(provider.label), providers: [] };
      vendors.push(vendor);
    }
    vendor.providers.push(provider);
  }
  for (const vendor of vendors) {
    vendor.providers.sort((a, b) => Number(b.kind === 'hosted') - Number(a.kind === 'hosted'));
  }
  return vendors;
}

/**
 * Which settings block a provider's voice/language pickers write to, and the
 * field names inside it. Hosted and BYOK Gemini share the `gemini` block.
 */
export function voiceFields(
  providerId: string
):
  | { block: 'gemini'; voice: 'voice'; language: 'language' }
  | { block: 'sarvam'; voice: 'speaker'; language: 'language' }
  | { block: 'elevenlabs'; voice: 'voice_id'; language: null }
  | null {
  if (providerId === 'gemini' || providerId === 'gemini-hosted') {
    return { block: 'gemini', voice: 'voice', language: 'language' };
  }
  if (providerId === 'sarvam') return { block: 'sarvam', voice: 'speaker', language: 'language' };
  if (providerId === 'elevenlabs-hosted') {
    return { block: 'elevenlabs', voice: 'voice_id', language: null };
  }
  return null;
}

export function readSetting(settings: LiveVoiceSettings, block: string, field: string): string {
  const values = (settings as unknown as Record<string, Record<string, string | null>>)[block];
  return values?.[field] ?? '';
}
