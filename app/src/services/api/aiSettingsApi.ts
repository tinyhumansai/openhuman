/**
 * AI settings façade for the unified Settings → AI panel.
 *
 * Sits between the panel's React state and the Rust JSON-RPC core. Three
 * orthogonal surfaces in one place:
 *
 *  1. Cloud providers + per-workload routing → `openhuman.inference_update_model_settings`
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
  type ModelRegistryEntry,
  type ModelSettingsUpdate,
  openhumanGetClientConfig,
  openhumanUpdateLocalAiSettings,
  openhumanUpdateModelSettings,
} from '../../utils/tauriCommands/config';
import {
  type InstalledModelInfo,
  type LocalAiDiagnostics,
  type LocalAiStatus,
  type ModelPresetResult,
  openhumanLocalAiApplyPreset,
  openhumanLocalAiDiagnostics,
  openhumanLocalAiPresets,
  openhumanLocalAiStatus,
  type PresetsResponse,
} from '../../utils/tauriCommands/localAi';

// ─── Domain types — what the AIPanel consumes ──────────────────────────────

export type WorkloadId =
  | 'chat'
  | 'reasoning'
  | 'agentic'
  | 'coding'
  | 'vision'
  | 'memory'
  | 'heartbeat'
  | 'learning'
  | 'subconscious';

export const CHAT_WORKLOADS: WorkloadId[] = ['chat', 'reasoning', 'agentic', 'coding'];
export const BACKGROUND_WORKLOADS: WorkloadId[] = [
  'memory',
  'heartbeat',
  'learning',
  'subconscious',
];
export const ALL_WORKLOADS: WorkloadId[] = [...CHAT_WORKLOADS, ...BACKGROUND_WORKLOADS];

// Workloads that own a `<id>_provider` config field and must round-trip through
// settings serialization. Includes the tier-specific `vision` workload, which
// is deliberately NOT part of `CHAT_WORKLOADS`/`ALL_WORKLOADS`: it defaults to
// the managed `vision-v1` tier and is a delegate (like agentic BYOK), so it does
// not participate in the billing-suppression / "routed away from OpenHuman"
// checks in `useUsageState`.
export const ROUTABLE_WORKLOADS: WorkloadId[] = [...ALL_WORKLOADS, 'vision'];
export const OPENAI_CODEX_OAUTH_MISSING_AUTH_URL = 'OPENAI_CODEX_OAUTH_MISSING_AUTH_URL';
export const OPENAI_CODEX_OAUTH_MISSING_CALLBACK_URL = 'OPENAI_CODEX_OAUTH_MISSING_CALLBACK_URL';

/** Provider reference parsed from a stored provider-string.
 *
 * Wire grammar: `"<slug>:<model>[@<temperature>]"`. The optional
 * `@<temperature>` suffix overrides the global default for this workload
 * only. The Rust factory strips it before sending the model id upstream.
 */
export type ProviderRef =
  | { kind: 'openhuman' }
  | { kind: 'default' }
  | { kind: 'cloud'; providerSlug: string; model: string; temperature?: number | null }
  | { kind: 'local'; model: string; temperature?: number | null }
  | { kind: 'claude-code'; model: string; temperature?: number | null };

/** Parse a `<model>[@<temp>]` suffix into `(model, temperature)`. */
function splitModelAndTemp(raw: string): { model: string; temperature: number | null } {
  const at = raw.lastIndexOf('@');
  if (at < 0) return { model: raw.trim(), temperature: null };
  const head = raw.slice(0, at).trim();
  const tail = raw.slice(at + 1).trim();
  const parsed = Number(tail);
  if (!head || !Number.isFinite(parsed)) {
    // Malformed suffix — treat the whole thing as the model id.
    return { model: raw.trim(), temperature: null };
  }
  return { model: head, temperature: parsed };
}

/** Format the model + optional temperature suffix used on the wire. */
function joinModelAndTemp(model: string, temperature: number | null | undefined): string {
  if (temperature == null || !Number.isFinite(temperature)) return model;
  // Two decimal places is plenty for the 0..2 slider and avoids 0.7000000001 drift.
  const rounded = Math.round(temperature * 100) / 100;
  return `${model}@${String(rounded)}`;
}

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

export interface ProviderModelTestResult {
  reply: string;
}

export interface OpenAiCodexOAuthStartResult {
  authUrl: string;
  state?: string;
  redirectUri?: string;
}

const PROVIDER_MODEL_TEST_TIMEOUT_MS = 120_000;

/** Single in-memory snapshot the AI panel renders against. */
export interface AISettings {
  cloudProviders: CloudProviderView[];
  routing: Record<WorkloadId, ProviderRef>;
  /**
   * Per-model registry carrying each model's user-set `vision` flag, keyed by
   * `(provider slug, model id)`. Surfaced in the custom-model dialog; gates chat
   * image attachments for custom/BYOK models.
   */
  modelRegistry: ModelRegistryEntry[];
  /**
   * #3767: authoritative, core-side per-tier decision (mirrors the Rust factory's
   * real routing resolution). For each chat-mode tier (`chat` = Quick mode,
   * `reasoning` = Reasoning mode), true when that tier runs on a non-managed
   * provider the user funds themselves (a usable BYO key, local runtime, or
   * claude-code). `useUsageState` checks the tier matching the selected mode so
   * the "buy credits" prompt is hidden exactly when the core says that mode does
   * not bill managed credits. Each entry is `false` when the core snapshot
   * predates this field (conservative — keep gating).
   *
   * Optional: `loadAISettings` always populates it, but AIPanel reconstructs a
   * draft `AISettings` for `saveAISettings` (which ignores this read-only field)
   * and test fixtures may omit it — consumers treat a missing entry as `false`.
   */
  creditsBypass?: { chat: boolean; reasoning: boolean };
}

/** Re-export so callers (e.g. the AI panel) can reference the entry type. */
export type { ModelRegistryEntry };

/** Find a model's vision flag in the registry, matching by (provider, id).
 *  Tolerates an undefined registry (older snapshots / transient load state). */
export function modelRegistryVision(
  registry: ModelRegistryEntry[] | undefined,
  provider: string,
  id: string
): boolean {
  return (registry ?? []).some(e => e.provider === provider && e.id === id && e.vision);
}

/**
 * Upsert a model's vision flag, returning a new array. Matches by
 * `(provider, id)`; a `vision: false` entry is removed (absence ⇒ no vision).
 */
export function upsertModelRegistryVision(
  registry: ModelRegistryEntry[] | undefined,
  provider: string,
  id: string,
  vision: boolean
): ModelRegistryEntry[] {
  const base = registry ?? [];
  const without = base.filter(e => !(e.provider === provider && e.id === id));
  if (!vision) {
    return without;
  }
  const existing = base.find(e => e.provider === provider && e.id === id);
  return [
    ...without,
    { id, provider, cost_per_1m_output: existing?.cost_per_1m_output ?? 0, vision: true },
  ];
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
  if (!trimmed || trimmed === 'cloud') {
    return { kind: 'default' };
  }
  if (trimmed === 'openhuman') {
    return { kind: 'openhuman' };
  }
  if (trimmed.startsWith('ollama:')) {
    const { model, temperature } = splitModelAndTemp(trimmed.slice('ollama:'.length));
    return temperature == null ? { kind: 'local', model } : { kind: 'local', model, temperature };
  }
  if (trimmed.startsWith('claude-code:')) {
    const { model, temperature } = splitModelAndTemp(trimmed.slice('claude-code:'.length));
    return temperature == null
      ? { kind: 'claude-code', model }
      : { kind: 'claude-code', model, temperature };
  }
  const colonIdx = trimmed.indexOf(':');
  if (colonIdx > 0) {
    const slug = trimmed.slice(0, colonIdx).trim();
    const { model, temperature } = splitModelAndTemp(trimmed.slice(colonIdx + 1));
    if (slug === 'openhuman') {
      return { kind: 'openhuman' };
    }
    return temperature == null
      ? { kind: 'cloud', providerSlug: slug, model }
      : { kind: 'cloud', providerSlug: slug, model, temperature };
  }
  // Unrecognised bare string → fall back to openhuman.
  return { kind: 'openhuman' };
}

/** Serialise a `ProviderRef` back to the wire-format string. */
export function serializeProviderRef(ref: ProviderRef): string {
  switch (ref.kind) {
    case 'openhuman':
      return 'openhuman';
    case 'default':
      return 'cloud';
    case 'cloud':
      return `${ref.providerSlug}:${joinModelAndTemp(ref.model, ref.temperature)}`;
    case 'local':
      return `ollama:${joinModelAndTemp(ref.model, ref.temperature)}`;
    case 'claude-code':
      return `claude-code:${joinModelAndTemp(ref.model, ref.temperature)}`;
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

  const cloudProviders: CloudProviderView[] = config.cloud_providers
    .filter(p => !['', 'cloud', 'openhuman', 'pid'].includes(p.slug.trim()))
    .map(p => {
      const newKey = authKeyForSlug(p.slug).toLowerCase();
      const legacyKey = p.slug.toLowerCase();
      const has_api_key = profileProviders.has(newKey) || profileProviders.has(legacyKey);
      return { ...p, has_api_key };
    });

  const routing: Record<WorkloadId, ProviderRef> = {
    chat: parseProviderString(config.chat_provider),
    reasoning: parseProviderString(config.reasoning_provider),
    agentic: parseProviderString(config.agentic_provider),
    coding: parseProviderString(config.coding_provider),
    vision: parseProviderString(config.vision_provider),
    memory: parseProviderString(config.memory_provider),
    heartbeat: parseProviderString(config.heartbeat_provider),
    learning: parseProviderString(config.learning_provider),
    subconscious: parseProviderString(config.subconscious_provider),
  };

  // Diagnostic: detect partial BYOK routing — some workloads have a BYOK cloud
  // provider configured while others are left at default/openhuman. The Rust
  // factory inherits the BYOK provider for unset workloads, but this log makes
  // it easy to trace the config state from the frontend side.
  const byokProvider = (['chat', 'reasoning', 'agentic', 'coding'] as const).find(w => {
    const ref_ = routing[w];
    return ref_.kind === 'cloud';
  });
  const hasUnsetChatWorkloads = (['chat', 'reasoning', 'coding'] as const).some(w => {
    const ref_ = routing[w];
    return ref_.kind === 'default';
  });
  if (byokProvider !== undefined && hasUnsetChatWorkloads) {
    const byokSlug = (routing[byokProvider] as { kind: 'cloud'; providerSlug: string })
      .providerSlug;
    console.debug(
      '[ai-settings] partial BYOK routing detected — unset workloads will inherit from: ' + byokSlug
    );
  }

  // Per-model registry (vision flags). Defensive default for older snapshots.
  const modelRegistry: ModelRegistryEntry[] = config.model_registry ?? [];

  // #3767: authoritative per-tier bypass flags from the core. Each entry
  // defaults to false for older snapshots that don't carry it (conservative —
  // keep the credits gate on).
  const creditsBypass = {
    chat: config.credits_bypass?.chat === true,
    reasoning: config.credits_bypass?.reasoning === true,
  };

  return { cloudProviders, routing, modelRegistry, creditsBypass };
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
    patch.cloud_providers = next.cloudProviders
      .filter(p => !['', 'cloud', 'openhuman', 'pid'].includes(p.slug.trim()))
      .map(({ id, slug, label, endpoint, auth_style }) => ({
        id,
        slug,
        label,
        endpoint,
        auth_style,
      }));
  }

  for (const w of ROUTABLE_WORKLOADS) {
    const a = serializeProviderRef(prev.routing[w]);
    const b = serializeProviderRef(next.routing[w]);
    if (a !== b) {
      patch[`${w}_provider` as keyof ModelSettingsUpdate] = b as never;
    }
  }

  // Per-model registry (vision flags): any change → send the full list.
  if (!modelRegistriesEqual(prev.modelRegistry, next.modelRegistry)) {
    patch.model_registry = next.modelRegistry.map(
      ({ id, provider, cost_per_1m_output, vision }) => ({
        id,
        provider,
        cost_per_1m_output,
        vision,
      })
    );
  }

  if (Object.keys(patch).length === 0) {
    return;
  }
  await openhumanUpdateModelSettings(patch);
}

/** Order-insensitive structural equality for two model registries. */
function modelRegistriesEqual(a: ModelRegistryEntry[], b: ModelRegistryEntry[]): boolean {
  if (a.length !== b.length) {
    return false;
  }
  const key = (e: ModelRegistryEntry) => `${e.provider} ${e.id}`;
  const bByKey = new Map(b.map(e => [key(e), e]));
  return a.every(e => {
    const m = bByKey.get(key(e));
    return !!m && m.vision === e.vision && m.cost_per_1m_output === e.cost_per_1m_output;
  });
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

export async function startOpenAiCodexOAuth(): Promise<OpenAiCodexOAuthStartResult> {
  const res = await callCoreRpc<{ result: OpenAiCodexOAuthStartResult }>({
    method: 'openhuman.inference_openai_oauth_start',
    params: {},
  });
  const authUrl = res?.result?.authUrl?.trim();
  if (!authUrl) {
    throw new Error(OPENAI_CODEX_OAUTH_MISSING_AUTH_URL);
  }
  return res.result;
}

export async function completeOpenAiCodexOAuth(callbackUrl: string): Promise<void> {
  const callback = callbackUrl.trim();
  if (!callback) {
    throw new Error(OPENAI_CODEX_OAUTH_MISSING_CALLBACK_URL);
  }
  await callCoreRpc({
    method: 'openhuman.inference_openai_oauth_complete',
    params: { callback_url: callback },
  });
}

export async function importOpenAiCodexCliAuth(): Promise<void> {
  await callCoreRpc({ method: 'openhuman.inference_openai_oauth_import_codex_cli', params: {} });
}

/**
 * Eagerly write the cloud_providers list to the core config.
 *
 * Called immediately when providers are added/edited/removed so that
 * `listProviderModels` can resolve the provider by id without waiting for
 * the user to click the global Save button.  API keys are NOT included here
 * (they're written via `setCloudProviderKey` on their own path).
 */
export async function flushCloudProviders(providers: CloudProviderCreds[]): Promise<void> {
  if (!isTauri()) return;
  await openhumanUpdateModelSettings({ cloud_providers: providers });
}

/**
 * Fetch the model list from a configured cloud provider's /models API.
 * `providerId` may be either the provider's opaque id or its slug — Rust
 * accepts both. Prefer passing the slug so lookup works before the provider
 * config has been persisted to disk (i.e. before the user clicks Save).
 * Throws on error so callers can surface retry UI. Returns [] when not
 * running in Tauri (browser dev mode has no RPC bridge).
 */
export async function listProviderModels(providerId: string): Promise<ModelInfo[]> {
  if (!isTauri()) {
    return [];
  }
  const res = await callCoreRpc<{ result: { models: ModelInfo[] } }>({
    method: 'openhuman.inference_list_models',
    params: { provider_id: providerId },
  });
  return res?.result?.models ?? [];
}

export async function testProviderModel(
  workload: WorkloadId,
  provider: string,
  prompt = 'Hello world'
): Promise<ProviderModelTestResult> {
  if (!isTauri()) {
    throw new Error('Model testing is only available in the desktop app.');
  }
  const res = await callCoreRpc<{ result: ProviderModelTestResult }>({
    method: 'openhuman.inference_test_provider_model',
    params: { workload, provider, prompt },
    timeoutMs: PROVIDER_MODEL_TEST_TIMEOUT_MS,
  });
  if (!res?.result) {
    throw new Error(
      `Model test RPC returned no result for ${workload} via ${provider} (openhuman.inference_test_provider_model).`
    );
  }
  return res.result;
}

// ─── Local provider façade (Ollama install / detect / model manage) ───────

/** Snapshot of the Ollama daemon + installed-model state for the AI panel. */
export interface LocalProviderSnapshot {
  status: LocalAiStatus | null;
  diagnostics: LocalAiDiagnostics | null;
  presets: PresetsResponse | null;
  installedModels: InstalledModelInfo[];
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

/** Convenience helpers re-exported so the panel imports from one place. */
export const localProvider = {
  applyPreset: (tier: string) => openhumanLocalAiApplyPreset(tier),
  setEnabled: (enabled: boolean) => setLocalRuntimeEnabled(enabled),
};

export type { ModelPresetResult };
