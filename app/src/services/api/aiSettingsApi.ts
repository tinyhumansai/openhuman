/**
 * AI settings façade for the unified Settings → AI panel.
 *
 * Sits between the panel's React state and the Rust JSON-RPC core. Three
 * orthogonal surfaces in one place:
 *
 *  1. Cloud providers + per-workload routing → `openhuman.update_model_settings`
 *  2. API keys for cloud providers           → `openhuman.auth_*_provider_credentials`
 *                                              (encrypted at rest in
 *                                              `auth-profiles.json`)
 *  3. Local provider (Ollama) status + models → existing `localAi.ts` exports
 *                                              (re-exported here for symmetry)
 *
 * The panel itself never imports `coreRpcClient` directly — every call goes
 * through this file. Keeps the wiring testable and the panel focused on
 * presentation.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { CORE_RPC_METHODS } from '../../services/rpcMethods';
import {
  authListProviderCredentials,
  type AuthProfileSummary,
  authRemoveProviderCredentials,
  authStoreProviderCredentials,
} from '../../utils/tauriCommands/auth';
import { isTauri } from '../../utils/tauriCommands/common';
import {
  type ClientConfig,
  type CloudProviderCreds,
  type ModelSettingsUpdate,
  openhumanGetClientConfig,
  openhumanUpdateLocalAiSettings,
  openhumanUpdateModelSettings,
} from '../../utils/tauriCommands/config';
import {
  type LocalAiDiagnostics,
  type LocalAiStatus,
  type ModelPresetResult,
  openhumanLocalAiApplyPreset,
  openhumanLocalAiDiagnostics,
  openhumanLocalAiDownload,
  openhumanLocalAiPresets,
  openhumanLocalAiSetOllamaPath,
  openhumanLocalAiShutdownOwned,
  openhumanLocalAiStatus,
  type PresetsResponse,
} from '../../utils/tauriCommands/localAi';

// ─── Domain types — what the AIPanel consumes ──────────────────────────────

export type WorkloadId =
  | 'reasoning'
  | 'agentic'
  | 'coding'
  | 'memory'
  | 'embeddings'
  | 'heartbeat'
  | 'learning'
  | 'subconscious';

export const CHAT_WORKLOADS: WorkloadId[] = ['reasoning', 'agentic', 'coding'];
export const BACKGROUND_WORKLOADS: WorkloadId[] = [
  'memory',
  'embeddings',
  'heartbeat',
  'learning',
  'subconscious',
];
export const ALL_WORKLOADS: WorkloadId[] = [...CHAT_WORKLOADS, ...BACKGROUND_WORKLOADS];

/** Provider reference parsed from a stored provider-string. */
export type ProviderRef =
  | { kind: 'openhuman' }
  | { kind: 'cloud'; providerSlug: string; model: string }
  | { kind: 'local'; model: string };

/**
 * Cloud provider entry as the UI sees it — endpoint config plus a derived
 * `has_api_key` flag (true when a key is stored in `auth-profiles.json`).
 */
export interface CloudProviderView extends CloudProviderCreds {
  has_api_key: boolean;
}

/** Model descriptor returned by providers_list_models. */
export interface ModelInfo {
  id: string;
  owned_by?: string | null;
  context_window?: number | null;
}

/** Single in-memory snapshot the AI panel renders against. */
export interface AISettings {
  cloudProviders: CloudProviderView[];
  routing: Record<WorkloadId, ProviderRef>;
}

// ─── Read path: load + parse ───────────────────────────────────────────────

/**
 * Parse a stored provider string (e.g. `"openai:gpt-4o"`) into a structured
 * ProviderRef. Empty/null/`"cloud"` → openhuman. Mirrors the Rust factory grammar.
 *
 * New grammar: `"<slug>:<model>"`. Legacy bare sentinels:
 *   - `"openhuman"` → { kind: 'openhuman' }
 *   - `"cloud"` or empty → { kind: 'openhuman' }
 *   - `"ollama:<model>"` → { kind: 'local', model }
 *   - `"<slug>:<model>"` → { kind: 'cloud', providerSlug: slug, model }
 */
export function parseProviderString(s: string | null | undefined): ProviderRef {
  const trimmed = (s ?? '').trim();
  if (!trimmed || trimmed === 'cloud' || trimmed === 'openhuman') {
    return { kind: 'openhuman' };
  }
  if (trimmed.startsWith('ollama:')) {
    return { kind: 'local', model: trimmed.slice('ollama:'.length).trim() };
  }
  const colonIdx = trimmed.indexOf(':');
  if (colonIdx > 0) {
    const slug = trimmed.slice(0, colonIdx).trim();
    const model = trimmed.slice(colonIdx + 1).trim();
    if (slug === 'openhuman') {
      return { kind: 'openhuman' };
    }
    return { kind: 'cloud', providerSlug: slug, model };
  }
  // Unrecognised bare string → fall back to openhuman.
  return { kind: 'openhuman' };
}

/** Serialise a `ProviderRef` back to the wire-format string. */
export function serializeProviderRef(ref: ProviderRef): string {
  switch (ref.kind) {
    case 'openhuman':
      return 'openhuman';
    case 'cloud':
      return `${ref.providerSlug}:${ref.model}`;
    case 'local':
      return `ollama:${ref.model}`;
  }
}

/**
 * Auth-profile key for a slug-keyed provider (matches Rust `auth_key_for_slug`).
 * Used to look up whether an API key is stored for a given provider.
 */
function authKeyForSlug(slug: string): string {
  return `provider:${slug}`;
}

/**
 * Loads the full AI settings view by joining:
 *  - the core's client-config snapshot (cloud_providers + *_provider fields)
 *  - the auth profiles list (to derive `has_api_key` per cloud provider)
 *
 * Defensive: a failed `auth_list` (e.g. brand-new workspace, no profiles
 * file yet) silently degrades to `has_api_key: false` for all entries so
 * the panel still renders.
 */
export async function loadAISettings(): Promise<AISettings> {
  const [configRes, profilesRes] = await Promise.all([
    openhumanGetClientConfig(),
    authListProviderCredentials().catch((): { result: AuthProfileSummary[] } => ({ result: [] })),
  ]);
  const config: ClientConfig = configRes.result;
  // Build a set of stored provider keys for has_api_key derivation.
  // Supports both new-style `provider:<slug>` and legacy bare `<slug>`.
  const profileProviders = new Set(
    profilesRes.result.map((p: AuthProfileSummary) => p.provider.toLowerCase())
  );

  const cloudProviders: CloudProviderView[] = config.cloud_providers.map(p => {
    const newKey = authKeyForSlug(p.slug).toLowerCase();
    const legacyKey = p.slug.toLowerCase();
    const has_api_key = profileProviders.has(newKey) || profileProviders.has(legacyKey);
    return { ...p, has_api_key };
  });

  const routing: Record<WorkloadId, ProviderRef> = {
    reasoning: parseProviderString(config.reasoning_provider),
    agentic: parseProviderString(config.agentic_provider),
    coding: parseProviderString(config.coding_provider),
    memory: parseProviderString(config.memory_provider),
    embeddings: parseProviderString(config.embeddings_provider),
    heartbeat: parseProviderString(config.heartbeat_provider),
    learning: parseProviderString(config.learning_provider),
    subconscious: parseProviderString(config.subconscious_provider),
  };

  return { cloudProviders, routing };
}

// ─── Write path: diff + save ───────────────────────────────────────────────

/**
 * Persist a draft `AISettings` to the core. Diffs against a previous snapshot
 * and only sends fields that actually changed — keeps the patch small and
 * avoids inadvertently overwriting unrelated fields edited elsewhere.
 */
export async function saveAISettings(prev: AISettings, next: AISettings): Promise<void> {
  const patch: ModelSettingsUpdate = {};

  // Cloud providers: any change → send the full list.
  if (
    prev.cloudProviders.length !== next.cloudProviders.length ||
    prev.cloudProviders.some((p, i) => {
      const n = next.cloudProviders[i];
      return (
        !n ||
        n.id !== p.id ||
        n.slug !== p.slug ||
        n.label !== p.label ||
        n.endpoint !== p.endpoint ||
        n.auth_style !== p.auth_style
      );
    })
  ) {
    patch.cloud_providers = next.cloudProviders.map(
      ({ id, slug, label, endpoint, auth_style }) => ({ id, slug, label, endpoint, auth_style })
    );
  }

  for (const w of ALL_WORKLOADS) {
    const a = serializeProviderRef(prev.routing[w]);
    const b = serializeProviderRef(next.routing[w]);
    if (a !== b) {
      patch[`${w}_provider` as keyof ModelSettingsUpdate] = b as never;
    }
  }

  if (Object.keys(patch).length === 0) {
    return;
  }
  await openhumanUpdateModelSettings(patch);
}

// ─── API key management (per cloud provider slug) ──────────────────────────

/**
 * Store an API key for a cloud provider (encrypted at rest). Keyed by slug
 * using the new `provider:<slug>` format.
 */
export async function setCloudProviderKey(slug: string, apiKey: string): Promise<void> {
  if (slug === 'openhuman') {
    throw new Error('OpenHuman uses the session JWT — keys are not configurable here.');
  }
  // Store under both new-style key `provider:<slug>` and legacy bare `<slug>`
  // so old code paths that look up by bare slug continue to work.
  await authStoreProviderCredentials({
    provider: authKeyForSlug(slug),
    profile: 'default',
    token: apiKey,
    setActive: true,
  });
}

/** Clear a stored API key. */
export async function clearCloudProviderKey(slug: string): Promise<void> {
  if (slug === 'openhuman') {
    return;
  }
  // Clear the new-style key. Legacy bare-slug entries are left as-is
  // since we can't be sure they aren't used by other things.
  await authRemoveProviderCredentials({ provider: authKeyForSlug(slug), profile: 'default' });
}

/**
 * Fetch the model list from a configured cloud provider's /models API.
 * Returns an empty array on error (callers should handle gracefully).
 */
export async function listProviderModels(providerId: string): Promise<ModelInfo[]> {
  if (!isTauri()) {
    return [];
  }
  try {
    const res = await callCoreRpc<{ result: { models: ModelInfo[] } }>({
      method: CORE_RPC_METHODS.providersListModels,
      params: { provider_id: providerId },
    });
    return res?.result?.models ?? [];
  } catch {
    return [];
  }
}

// ─── Local provider façade (Ollama install / detect / model manage) ───────

/** Snapshot of the Ollama daemon + installed-model state for the AI panel. */
export interface LocalProviderSnapshot {
  status: LocalAiStatus | null;
  diagnostics: LocalAiDiagnostics | null;
  presets: PresetsResponse | null;
  installedModels: Array<{ name: string; size?: number | null }>;
}

export async function loadLocalProviderSnapshot(): Promise<LocalProviderSnapshot> {
  const [statusRes, diag, presets] = await Promise.all([
    openhumanLocalAiStatus().catch((): { result: LocalAiStatus | null } => ({ result: null })),
    openhumanLocalAiDiagnostics().catch((): LocalAiDiagnostics | null => null),
    openhumanLocalAiPresets().catch((): PresetsResponse | null => null),
  ]);
  return {
    status: statusRes.result,
    diagnostics: diag,
    presets,
    installedModels: diag?.installed_models ?? [],
  };
}

/**
 * Toggle the master local-AI runtime (Ollama daemon orchestration). When
 * `false`, every workload routed to `ollama:*` will fail to build at the
 * factory level — the user should leave routes set to "openhuman" while local
 * AI is disabled. The new AI panel surfaces this as a single switch.
 *
 * Critically: this flips BOTH `runtime_enabled` AND `opt_in_confirmed`.
 */
export async function setLocalRuntimeEnabled(enabled: boolean): Promise<void> {
  await openhumanUpdateLocalAiSettings({ runtime_enabled: enabled, opt_in_confirmed: enabled });
}

/**
 * Set / clear the user-configured Ollama binary path.
 */
export async function setLocalOllamaPath(path: string): Promise<void> {
  await openhumanLocalAiSetOllamaPath(path);
}

/**
 * Gate off the local-AI runtime.
 */
export async function shutdownLocalProvider(): Promise<void> {
  await setLocalRuntimeEnabled(false);
  await openhumanLocalAiShutdownOwned();
}

/** Convenience helpers re-exported so the panel imports from one place. */
export const localProvider = {
  applyPreset: (tier: string) => openhumanLocalAiApplyPreset(tier),
  download: (retry: boolean) => openhumanLocalAiDownload(retry),
  setEnabled: (enabled: boolean) => setLocalRuntimeEnabled(enabled),
  setBinaryPath: (path: string) => setLocalOllamaPath(path),
  shutdown: () => shutdownLocalProvider(),
};

export type { ModelPresetResult };
