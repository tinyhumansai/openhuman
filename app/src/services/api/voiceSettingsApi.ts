/**
 * Voice settings facade for the Settings > Voice panel.
 *
 * Mirrors aiSettingsApi.ts for the voice provider registry:
 *  1. Voice providers + STT/TTS routing -> `openhuman.voice_update_provider_settings`
 *  2. API keys for voice providers      -> `openhuman.auth_*_provider_credentials`
 *                                          (shared namespace with LLM providers)
 *  3. Model/voice listings              -> `openhuman.voice_list_models`
 *  4. Provider testing                  -> `openhuman.voice_test_provider`
 */
import {
  authListProviderCredentials,
  type AuthProfileSummary,
  authRemoveProviderCredentials,
  authStoreProviderCredentials,
} from '../../utils/tauriCommands/auth';
import { callCoreRpc } from '../coreRpcClient';

// ---- Domain types ----

export type VoiceWorkloadId = 'stt' | 'tts';

/**
 * Structured reference to a voice provider. Parsed from wire-format strings.
 *
 * Wire grammar:
 *   "cloud" / "openhuman" / empty -> { kind: 'cloud' }
 *   "whisper"                     -> { kind: 'local', engine: 'whisper', model }
 *   "piper"                       -> { kind: 'local', engine: 'piper', model }
 *   "<slug>:<model>"              -> { kind: 'external', providerSlug, model }
 */
export type VoiceProviderRef =
  | { kind: 'cloud' }
  | { kind: 'local'; engine: 'whisper' | 'piper'; model: string }
  | { kind: 'external'; providerSlug: string; model: string };

export type VoiceCapability = 'stt' | 'tts' | 'both';

export interface VoiceProviderCreds {
  id: string;
  slug: string;
  label: string;
  endpoint: string;
  auth_style: string;
  capability: VoiceCapability;
  stt_api_style: string;
  tts_api_style: string;
  default_stt_model: string | null;
  default_tts_voice: string | null;
}

export interface VoiceProviderView extends VoiceProviderCreds {
  has_api_key: boolean;
}

export interface VoiceModelInfo {
  id: string;
  label?: string | null;
}

export interface VoiceTestResult {
  ok: boolean;
  detail: string;
  latency_ms?: number;
}

export interface VoiceSettings {
  voiceProviders: VoiceProviderView[];
  sttProvider: VoiceProviderRef;
  ttsProvider: VoiceProviderRef;
}

// ---- Parse / Serialize ----

/**
 * Parse a stored voice provider string into a structured VoiceProviderRef.
 * Mirrors the Rust voice factory grammar.
 */
export function parseVoiceProviderString(s: string | null | undefined): VoiceProviderRef {
  const trimmed = (s ?? '').trim();
  if (!trimmed || trimmed === 'cloud' || trimmed === 'openhuman') {
    return { kind: 'cloud' };
  }
  if (trimmed === 'whisper') {
    return { kind: 'local', engine: 'whisper', model: '' };
  }
  if (trimmed === 'piper') {
    return { kind: 'local', engine: 'piper', model: '' };
  }
  const colonIdx = trimmed.indexOf(':');
  if (colonIdx > 0) {
    const slug = trimmed.slice(0, colonIdx).trim();
    const model = trimmed.slice(colonIdx + 1).trim();
    if (slug === 'whisper') {
      return { kind: 'local', engine: 'whisper', model };
    }
    if (slug === 'piper') {
      return { kind: 'local', engine: 'piper', model };
    }
    return { kind: 'external', providerSlug: slug, model };
  }
  return { kind: 'cloud' };
}

/** Serialize a VoiceProviderRef back to the wire-format string. */
export function serializeVoiceProviderRef(ref: VoiceProviderRef): string {
  switch (ref.kind) {
    case 'cloud':
      return 'cloud';
    case 'local':
      return ref.model ? `${ref.engine}:${ref.model}` : ref.engine;
    case 'external':
      return ref.model ? `${ref.providerSlug}:${ref.model}` : ref.providerSlug;
  }
}

/**
 * Auth-profile key for a slug-keyed provider. Matches Rust `auth_key_for_slug`.
 * Shared namespace with LLM providers — an `openai` key works for both.
 */
function authKeyForSlug(slug: string): string {
  return `provider:${slug}`;
}

// ---- Read path ----

/**
 * Load voice settings by joining the core's config snapshot with auth profiles
 * to derive `has_api_key` per provider.
 */
export async function loadVoiceSettings(): Promise<VoiceSettings> {
  // The client config RPC returns a large object; we only need the voice
  // fields. Cast via `unknown` to avoid TS structural overlap complaints.
  const [configResult, profilesRes] = await Promise.all([
    callCoreRpc<Record<string, unknown>>({
      method: 'openhuman.config_get_client_config',
      params: {},
    }).then(raw => {
      // The config_get_client_config RPC wraps its payload in { result: ... }.
      const cfg = (raw as { result?: Record<string, unknown> }).result ?? raw;
      return {
        voice_providers: (cfg.voice_providers as VoiceProviderCreds[] | undefined) ?? [],
        stt_provider: (cfg.stt_provider as string | null | undefined) ?? null,
        tts_provider: (cfg.tts_provider as string | null | undefined) ?? null,
      };
    }),
    authListProviderCredentials().catch((): { result: AuthProfileSummary[] } => ({ result: [] })),
  ]);

  const providers = configResult.voice_providers;
  const authSlugs = new Set(
    profilesRes.result.map((p: AuthProfileSummary) => p.provider.toLowerCase())
  );

  const voiceProviders: VoiceProviderView[] = providers.map(p => ({
    ...p,
    has_api_key:
      authSlugs.has(authKeyForSlug(p.slug).toLowerCase()) || authSlugs.has(p.slug.toLowerCase()),
  }));

  if (process.env.NODE_ENV !== 'production') {
    console.debug('[voiceSettingsApi] loaded', {
      providerCount: voiceProviders.length,
      slugs: voiceProviders.map(p => p.slug),
      stt: configResult.stt_provider,
      tts: configResult.tts_provider,
    });
  }

  return {
    voiceProviders,
    sttProvider: parseVoiceProviderString(configResult.stt_provider),
    ttsProvider: parseVoiceProviderString(configResult.tts_provider),
  };
}

// ---- Write path ----

interface VoiceProviderSettingsUpdate {
  voice_providers?: VoiceProviderCreds[];
  stt_provider?: string;
  tts_provider?: string;
}

/**
 * Save voice settings by diffing against the previous snapshot and only
 * patching changed fields.
 */
export async function saveVoiceSettings(prev: VoiceSettings, next: VoiceSettings): Promise<void> {
  const patch: VoiceProviderSettingsUpdate = {};
  let hasChanges = false;

  const prevProviderJson = JSON.stringify(prev.voiceProviders.map(stripHasKey));
  const nextProviderJson = JSON.stringify(next.voiceProviders.map(stripHasKey));
  if (prevProviderJson !== nextProviderJson) {
    patch.voice_providers = next.voiceProviders.map(stripHasKey);
    hasChanges = true;
  }

  const prevStt = serializeVoiceProviderRef(prev.sttProvider);
  const nextStt = serializeVoiceProviderRef(next.sttProvider);
  if (prevStt !== nextStt) {
    patch.stt_provider = nextStt;
    hasChanges = true;
  }

  const prevTts = serializeVoiceProviderRef(prev.ttsProvider);
  const nextTts = serializeVoiceProviderRef(next.ttsProvider);
  if (prevTts !== nextTts) {
    patch.tts_provider = nextTts;
    hasChanges = true;
  }

  if (!hasChanges) return;

  if (process.env.NODE_ENV !== 'production') {
    console.debug('[voiceSettingsApi] saving patch', patch);
  }

  await callCoreRpc({ method: 'openhuman.voice_update_provider_settings', params: patch });
}

function stripHasKey(p: VoiceProviderView): VoiceProviderCreds {
  const { has_api_key: _, ...rest } = p;
  return rest;
}

// ---- Model listing ----

export async function listVoiceModels(
  providerId: string,
  capability?: VoiceWorkloadId
): Promise<VoiceModelInfo[]> {
  const result = await callCoreRpc<{ models: VoiceModelInfo[] }>({
    method: 'openhuman.voice_list_models',
    params: { provider_id: providerId, capability },
  });
  return result.models ?? [];
}

// ---- Provider testing ----

const VOICE_TEST_TIMEOUT_MS = 30_000;

function stripLogPrefix(s: string): string {
  return s.replace(/^\[voice-(?:stt|tts|factory)\]\s*/i, '');
}

export async function testVoiceProvider(
  workload: VoiceWorkloadId,
  provider: string,
  validateOnly = false
): Promise<VoiceTestResult> {
  const result = await callCoreRpc<VoiceTestResult>({
    method: 'openhuman.voice_test_provider',
    params: { workload, provider, validate_only: validateOnly },
    timeoutMs: VOICE_TEST_TIMEOUT_MS,
  });
  return { ...result, detail: stripLogPrefix(result.detail) };
}

// ---- API key management (shared with LLM providers) ----

export async function setVoiceProviderKey(slug: string, apiKey: string): Promise<void> {
  await authStoreProviderCredentials({ provider: authKeyForSlug(slug), token: apiKey });
}

export async function clearVoiceProviderKey(slug: string): Promise<void> {
  await authRemoveProviderCredentials({ provider: authKeyForSlug(slug) });
}
