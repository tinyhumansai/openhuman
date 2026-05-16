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
import {
  authListProviderCredentials,
  type AuthProfileSummary,
  authRemoveProviderCredentials,
  authStoreProviderCredentials,
} from '../../utils/tauriCommands/auth';
import {
  type ClientConfig,
  type CloudProviderCreds,
  type CloudProviderType,
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
  | { kind: 'primary' }
  | { kind: 'cloud'; providerType: CloudProviderType; model: string }
  | { kind: 'local'; model: string };

/**
 * Cloud provider entry as the UI sees it — endpoint config plus a derived
 * `has_api_key` flag (true when a key is stored in `auth-profiles.json`).
 */
export interface CloudProviderView extends CloudProviderCreds {
  has_api_key: boolean;
}

/** Single in-memory snapshot the AI panel renders against. */
export interface AISettings {
  cloudProviders: CloudProviderView[];
  primaryCloudId: string | null;
  routing: Record<WorkloadId, ProviderRef>;
}

// ─── Read path: load + parse ───────────────────────────────────────────────

const PROVIDER_PREFIXES: Record<string, CloudProviderType> = {
  openhuman: 'openhuman',
  openai: 'openai',
  anthropic: 'anthropic',
  openrouter: 'openrouter',
  custom: 'custom',
};

/**
 * Parse a stored provider string (e.g. `"openai:gpt-4o"`) into a structured
 * ProviderRef. Empty/null/`"cloud"` → primary. Unrecognised → primary (safest
 * fallback). Mirrors the Rust factory grammar.
 */
export function parseProviderString(s: string | null | undefined): ProviderRef {
  const trimmed = (s ?? '').trim();
  if (!trimmed || trimmed === 'cloud') {
    return { kind: 'primary' };
  }
  if (trimmed.startsWith('ollama:')) {
    return { kind: 'local', model: trimmed.slice('ollama:'.length).trim() };
  }
  // Bare "openhuman" (no model suffix) means "use the OpenHuman backend with
  // its default model" — map to a cloud ref so the round-trip preserves the
  // explicit override rather than collapsing to the primary-cloud sentinel.
  if (trimmed === 'openhuman') {
    return { kind: 'cloud', providerType: 'openhuman', model: '' };
  }
  for (const prefix of Object.keys(PROVIDER_PREFIXES)) {
    if (trimmed.startsWith(`${prefix}:`)) {
      return {
        kind: 'cloud',
        providerType: PROVIDER_PREFIXES[prefix],
        model: trimmed.slice(prefix.length + 1).trim(),
      };
    }
  }
  return { kind: 'primary' };
}

/** Serialise a `ProviderRef` back to the wire-format string. */
export function serializeProviderRef(ref: ProviderRef): string {
  switch (ref.kind) {
    case 'primary':
      return 'cloud';
    case 'cloud':
      // Bare "openhuman" (no model) uses the sentinel form the Rust factory
      // expects — avoid emitting "openhuman:" (with trailing colon) which the
      // factory does not parse.
      if (ref.providerType === 'openhuman' && !ref.model) {
        return 'openhuman';
      }
      return `${ref.providerType}:${ref.model}`;
    case 'local':
      return `ollama:${ref.model}`;
  }
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
  const profilesByProvider = new Set(
    profilesRes.result.map((p: AuthProfileSummary) => p.provider.toLowerCase())
  );

  const cloudProviders: CloudProviderView[] = config.cloud_providers.map(p => ({
    ...p,
    has_api_key: profilesByProvider.has(p.type.toLowerCase()),
  }));

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

  return { cloudProviders, primaryCloudId: config.primary_cloud, routing };
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
        n.type !== p.type ||
        n.endpoint !== p.endpoint ||
        n.default_model !== p.default_model
      );
    })
  ) {
    patch.cloud_providers = next.cloudProviders.map(({ id, type, endpoint, default_model }) => ({
      id,
      type,
      endpoint,
      default_model,
    }));
  }

  if (prev.primaryCloudId !== next.primaryCloudId) {
    patch.primary_cloud = next.primaryCloudId ?? '';
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

// ─── API key management (per cloud provider type) ──────────────────────────

/**
 * Store an API key for a cloud provider (encrypted at rest). The provider
 * type doubles as the auth-profile id, so every cloud_providers entry of
 * the same type shares the same key.
 */
export async function setCloudProviderKey(
  providerType: CloudProviderType,
  apiKey: string
): Promise<void> {
  if (providerType === 'openhuman') {
    throw new Error('OpenHuman uses the session JWT — keys are not configurable here.');
  }
  await authStoreProviderCredentials({
    provider: providerType,
    profile: 'default',
    token: apiKey,
    setActive: true,
  });
}

// ─── Provider key validation ──────────────────────────────────────────────
//
// Sanity-checks an API key by hitting the provider's "list models" endpoint.
// Returns `ok: true` plus the model count on success, or `ok: false` with a
// short error string on failure. Best-effort: providers we don't know how
// to validate against (custom / local runtimes) resolve to `{ ok: true }`
// without making a request, since "no failure" is the most useful default.

export interface ProviderValidationResult {
  ok: boolean;
  /** Number of models the endpoint returned on success, when available. */
  modelCount?: number;
  /** Sorted model IDs returned by the endpoint, when parseable. Used by
   *  the custom-routing dialog to populate a model dropdown without
   *  having to round-trip back through the core for the decrypted key. */
  modelIds?: string[];
  /** Short human-readable failure reason on `ok: false`. */
  error?: string;
}

interface ValidationEndpoint {
  url: string;
  authHeader: (apiKey: string) => Record<string, string>;
  /** Extract the models list from the parsed JSON response, if shaped. */
  extractList?: (json: unknown) => unknown[] | null;
}

const PROVIDER_VALIDATION: Partial<Record<CloudProviderType, ValidationEndpoint>> = {
  openai: {
    url: 'https://api.openai.com/v1/models',
    authHeader: key => ({ Authorization: `Bearer ${key}` }),
    extractList: json =>
      typeof json === 'object' &&
      json &&
      'data' in json &&
      Array.isArray((json as Record<string, unknown>).data)
        ? ((json as Record<string, unknown>).data as unknown[])
        : null,
  },
  anthropic: {
    url: 'https://api.anthropic.com/v1/models',
    authHeader: key => ({ 'x-api-key': key, 'anthropic-version': '2023-06-01' }),
    extractList: json =>
      typeof json === 'object' &&
      json &&
      'data' in json &&
      Array.isArray((json as Record<string, unknown>).data)
        ? ((json as Record<string, unknown>).data as unknown[])
        : null,
  },
  openrouter: {
    url: 'https://openrouter.ai/api/v1/models',
    authHeader: key => ({ Authorization: `Bearer ${key}` }),
    extractList: json =>
      typeof json === 'object' &&
      json &&
      'data' in json &&
      Array.isArray((json as Record<string, unknown>).data)
        ? ((json as Record<string, unknown>).data as unknown[])
        : null,
  },
};

export async function validateCloudProviderKey(
  providerType: CloudProviderType,
  apiKey: string
): Promise<ProviderValidationResult> {
  const cfg = PROVIDER_VALIDATION[providerType];
  if (!cfg) {
    // Custom / local-runtime entries don't have a known models endpoint;
    // accept the key as-is.
    return { ok: true };
  }
  try {
    const res = await fetch(cfg.url, { method: 'GET', headers: cfg.authHeader(apiKey) });
    if (!res.ok) {
      if (res.status === 401 || res.status === 403) {
        return { ok: false, error: 'Key rejected — check it and try again.' };
      }
      return { ok: false, error: `Models endpoint returned ${res.status} ${res.statusText}.` };
    }
    let modelCount: number | undefined;
    let modelIds: string[] | undefined;
    try {
      const json = (await res.json()) as unknown;
      const list = cfg.extractList?.(json);
      if (Array.isArray(list)) {
        modelCount = list.length;
        const ids = list
          .map(entry => {
            if (typeof entry === 'string') return entry;
            if (entry && typeof entry === 'object' && 'id' in (entry as Record<string, unknown>)) {
              const id = (entry as Record<string, unknown>).id;
              if (typeof id === 'string') return id;
            }
            return null;
          })
          .filter((id): id is string => typeof id === 'string' && id.length > 0);
        if (ids.length > 0) modelIds = ids.sort();
      }
    } catch {
      // Body parse failed — the 2xx alone is good enough.
    }
    return { ok: true, modelCount, modelIds };
  } catch (err) {
    return { ok: false, error: err instanceof Error ? err.message : String(err) };
  }
}

// ─── Provider model-id cache ─────────────────────────────────────────────
//
// `validateCloudProviderKey` is the only path where the renderer has the
// plaintext key in hand, so it doubles as the source of truth for the
// per-provider model dropdown shown in the custom-routing dialog. Cache
// the IDs here (localStorage) so the dropdown is populated immediately on
// subsequent loads without re-prompting the user for their key.

const MODEL_CACHE_PREFIX = 'openhuman.provider_model_ids.v1.';

function modelCacheKey(providerType: CloudProviderType): string {
  return `${MODEL_CACHE_PREFIX}${providerType}`;
}

export function cacheProviderModelIds(providerType: CloudProviderType, modelIds: string[]): void {
  if (typeof localStorage === 'undefined') return;
  try {
    localStorage.setItem(modelCacheKey(providerType), JSON.stringify(modelIds));
  } catch {
    // Storage full / blocked — caching is best-effort.
  }
}

export function loadProviderModelIds(providerType: CloudProviderType): string[] {
  if (typeof localStorage === 'undefined') return [];
  try {
    const raw = localStorage.getItem(modelCacheKey(providerType));
    if (!raw) return [];
    const parsed = JSON.parse(raw) as unknown;
    if (Array.isArray(parsed)) {
      return parsed.filter((s): s is string => typeof s === 'string' && s.length > 0);
    }
  } catch {
    // Bad JSON / parse failure — fall through.
  }
  return [];
}

export function clearProviderModelIds(providerType: CloudProviderType): void {
  if (typeof localStorage === 'undefined') return;
  try {
    localStorage.removeItem(modelCacheKey(providerType));
  } catch {
    // best-effort
  }
}

/** Clear a stored API key. */
export async function clearCloudProviderKey(providerType: CloudProviderType): Promise<void> {
  if (providerType === 'openhuman') {
    return;
  }
  await authRemoveProviderCredentials({ provider: providerType, profile: 'default' });
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
 * factory level — the user should leave routes set to "cloud" while local
 * AI is disabled. The new AI panel surfaces this as a single switch.
 *
 * Critically: this flips BOTH `runtime_enabled` AND `opt_in_confirmed`.
 * Bootstrap has a separate hard-override that forces status to "disabled"
 * whenever `opt_in_confirmed` is false, regardless of `runtime_enabled`.
 * Setting only `runtime_enabled = true` would spawn the daemon and
 * immediately get re-disabled on the next bootstrap call:
 *   [local_ai] bootstrap: opt_in_confirmed=false, hard-overriding to disabled
 * Tying the two flags together matches `apply_preset`'s behaviour and gives
 * the user a single-click enable.
 */
export async function setLocalRuntimeEnabled(enabled: boolean): Promise<void> {
  await openhumanUpdateLocalAiSettings({ runtime_enabled: enabled, opt_in_confirmed: enabled });
}

/**
 * Set / clear the user-configured Ollama binary path. The Rust resolver
 * tries (in order): this field → `OLLAMA_BIN` env → workspace bin →
 * system PATH → auto-install. Pass an empty string to clear and fall
 * back to auto-detection.
 *
 * Triggers a re-bootstrap on the Rust side so the new path takes effect
 * without needing a manual restart.
 */
export async function setLocalOllamaPath(path: string): Promise<void> {
  await openhumanLocalAiSetOllamaPath(path);
}

/**
 * Gate off the local-AI runtime: writes `runtime_enabled = false`, kills the
 * Ollama daemon ONLY if OpenHuman spawned it (external daemons are left
 * untouched), and forces status to `"disabled"`. Workloads routed to
 * `ollama:<model>` will fail at factory build time after this — the gate is
 * at the routing layer, not by killing arbitrary processes.
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
