/**
 * Live voice agent settings facade (Connections → Voice agents).
 *
 *  1. Provider catalogue          -> `openhuman.voice_live_providers`
 *  2. Settings read / patch       -> `openhuman.voice_live_settings_get|set`
 *  3. Connectivity probe          -> `openhuman.voice_live_test_provider`
 *  4. BYOK API keys               -> `openhuman.auth_*_provider_credentials`
 *                                    (`provider:<key_slug>`, shared with LLM/voice)
 *
 * The session itself runs over the core's `/ws/live-voice` socket — see
 * `features/human/voice/live/liveVoiceSocket.ts`.
 */
import {
  authRemoveProviderCredentials,
  authStoreProviderCredentials,
} from '../../utils/tauriCommands/auth';
import { callCoreRpc } from '../coreRpcClient';

export type LiveVoiceProviderId = 'gemini-hosted' | 'elevenlabs-hosted' | 'gemini' | 'sarvam';

export interface LiveVoiceProvider {
  id: LiveVoiceProviderId | string;
  label: string;
  kind: 'hosted' | 'byok';
  configured: boolean;
  key_slug: 'google' | 'sarvam' | string | null;
  voices: string[];
  languages: string[];
}

export interface LiveVoiceProviders {
  default_provider: string;
  providers: LiveVoiceProvider[];
}

export interface LiveVoiceSettings {
  default_provider: string | null;
  gemini: { model: string | null; voice: string | null; language: string | null };
  sarvam: { language: string | null; speaker: string | null; model: string | null };
  elevenlabs: { voice_id: string | null };
}

export interface LiveVoiceSettingsPatch {
  default_provider?: string;
  gemini?: Partial<LiveVoiceSettings['gemini']>;
  sarvam?: Partial<LiveVoiceSettings['sarvam']>;
  elevenlabs?: Partial<LiveVoiceSettings['elevenlabs']>;
}

export interface LiveVoiceTestResult {
  ok: boolean;
  latency_ms: number | null;
  error: string | null;
}

/** A provider probe dials the real provider, so give it longer than a plain RPC. */
const LIVE_VOICE_TEST_TIMEOUT_MS = 30_000;

/** Fill missing blocks so callers can read nested fields without guards. */
function normalizeSettings(raw: Partial<LiveVoiceSettings> | null | undefined): LiveVoiceSettings {
  return {
    default_provider: raw?.default_provider ?? null,
    gemini: {
      model: raw?.gemini?.model ?? null,
      voice: raw?.gemini?.voice ?? null,
      language: raw?.gemini?.language ?? null,
    },
    sarvam: {
      language: raw?.sarvam?.language ?? null,
      speaker: raw?.sarvam?.speaker ?? null,
      model: raw?.sarvam?.model ?? null,
    },
    elevenlabs: { voice_id: raw?.elevenlabs?.voice_id ?? null },
  };
}

export async function fetchLiveVoiceProviders(): Promise<LiveVoiceProviders> {
  const result = await callCoreRpc<Partial<LiveVoiceProviders>>({
    method: 'openhuman.voice_live_providers',
  });
  return {
    default_provider: result?.default_provider ?? '',
    providers: (result?.providers ?? []).map(p => ({
      ...p,
      voices: p.voices ?? [],
      languages: p.languages ?? [],
    })),
  };
}

export async function fetchLiveVoiceSettings(): Promise<LiveVoiceSettings> {
  const result = await callCoreRpc<Partial<LiveVoiceSettings>>({
    method: 'openhuman.voice_live_settings_get',
  });
  return normalizeSettings(result);
}

export async function updateLiveVoiceSettings(
  patch: LiveVoiceSettingsPatch
): Promise<LiveVoiceSettings> {
  const result = await callCoreRpc<Partial<LiveVoiceSettings>>({
    method: 'openhuman.voice_live_settings_set',
    params: patch as Record<string, unknown>,
  });
  return normalizeSettings(result);
}

export async function testLiveVoiceProvider(provider: string): Promise<LiveVoiceTestResult> {
  const result = await callCoreRpc<Partial<LiveVoiceTestResult>>({
    method: 'openhuman.voice_live_test_provider',
    params: { provider },
    timeoutMs: LIVE_VOICE_TEST_TIMEOUT_MS,
  });
  return {
    ok: Boolean(result?.ok),
    latency_ms: typeof result?.latency_ms === 'number' ? result.latency_ms : null,
    error: result?.error ?? null,
  };
}

/** Store a BYOK key under the shared `provider:<slug>` credential namespace. */
export async function saveLiveVoiceProviderKey(keySlug: string, apiKey: string): Promise<void> {
  await authStoreProviderCredentials({ provider: `provider:${keySlug}`, token: apiKey.trim() });
}

export async function clearLiveVoiceProviderKey(keySlug: string): Promise<void> {
  await authRemoveProviderCredentials({ provider: `provider:${keySlug}` });
}
