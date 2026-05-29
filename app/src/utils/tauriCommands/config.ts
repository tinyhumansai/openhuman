/**
 * Config and settings commands.
 */
import debug from 'debug';

import { callCoreRpc } from '../../services/coreRpcClient';
import { CORE_RPC_METHODS } from '../../services/rpcMethods';
import { CommandResponse, isTauri, tauriErrorMessage } from './common';

const log = debug('composio:rpc');

export interface ConfigSnapshot {
  config: Record<string, unknown>;
  workspace_dir: string;
  config_path: string;
}

export interface ModelRoute {
  hint: string;
  model: string;
}

/** Authentication header style. Matches Rust AuthStyle enum. */
export type AuthStyle = 'bearer' | 'anthropic' | 'openhuman_jwt' | 'none';

/** @deprecated Use AuthStyle. Kept for back-compat with old wire format. */
export type CloudProviderType =
  | 'openhuman'
  | 'openai'
  | 'anthropic'
  | 'openrouter'
  | 'orcarouter'
  | 'custom';

/**
 * Endpoint config for one cloud LLM provider (new slug-keyed shape).
 * API keys are NOT carried here — they live in `auth-profiles.json`
 * (set/cleared through the `auth_*` RPCs, keyed by `provider:<slug>`).
 */
export interface CloudProviderCreds {
  /** Opaque stable id, e.g. `"p_openai_a8c3f"`. Never shown in UI. */
  id: string;
  /** User-chosen routing key, e.g. `"openai"`. Used in `"<slug>:<model>"` strings. */
  slug: string;
  /** Human-readable display label, e.g. `"OpenAI"`. */
  label: string;
  endpoint: string;
  auth_style: AuthStyle;
}

export interface ModelSettingsUpdate {
  /**
   * OpenHuman product backend URL. Almost always left untouched; the
   * inference endpoint is the separate `inference_url` field.
   */
  api_url?: string | null;
  /**
   * Custom OpenAI-compatible LLM endpoint. When set together with
   * `api_key`, inference talks directly to this URL instead of routing
   * through the OpenHuman backend. Send an empty string to clear.
   */
  inference_url?: string | null;
  api_key?: string | null;
  default_model?: string | null;
  default_temperature?: number | null;
  /**
   * When present, REPLACES `config.model_routes` wholesale with these
   * `(hint, model)` pairs. Send `[]` to clear all routes (used when switching
   * back to the OpenHuman backend whose built-in router picks per-task models
   * on its own). Omit to leave existing routes untouched.
   */
  model_routes?: ModelRoute[] | null;
  /**
   * When present, REPLACES `config.cloud_providers` wholesale. API keys are
   * NOT carried here — store them via `authStoreProviderCredentials`.
   * Each entry: { id?, slug, label?, endpoint, auth_style? }
   */
  cloud_providers?: CloudProviderCreds[] | null;
  /** @deprecated No longer used — slug-based routing replaces primary_cloud. */
  primary_cloud?: string | null;
  /** Per-workload provider strings — see Rust `providers::factory` grammar. */
  chat_provider?: string | null;
  reasoning_provider?: string | null;
  agentic_provider?: string | null;
  coding_provider?: string | null;
  memory_provider?: string | null;
  embeddings_provider?: string | null;
  heartbeat_provider?: string | null;
  learning_provider?: string | null;
  subconscious_provider?: string | null;
}

/**
 * Stepped user-facing memory-context window preset. Mirrors the core
 * `MemoryContextWindow` enum (`src/openhuman/config/schema/agent.rs`)
 * — the actual char budgets are owned by the core, this is the label.
 */
export type MemoryContextWindow = 'minimal' | 'balanced' | 'extended' | 'maximum';

export const MEMORY_CONTEXT_WINDOWS: MemoryContextWindow[] = [
  'minimal',
  'balanced',
  'extended',
  'maximum',
];

export interface MemorySettingsUpdate {
  backend?: string | null;
  auto_save?: boolean | null;
  embedding_provider?: string | null;
  embedding_model?: string | null;
  embedding_dimensions?: number | null;
  /** One of `MEMORY_CONTEXT_WINDOWS`. */
  memory_window?: MemoryContextWindow | null;
}

export interface RuntimeSettingsUpdate {
  kind?: string | null;
  reasoning_enabled?: boolean | null;
}

export interface BrowserSettingsUpdate {
  enabled?: boolean | null;
}

export interface ScreenIntelligenceSettingsUpdate {
  enabled?: boolean | null;
  capture_policy?: string | null;
  policy_mode?: 'all_except_blacklist' | 'whitelist_only' | null;
  baseline_fps?: number | null;
  vision_enabled?: boolean | null;
  autocomplete_enabled?: boolean | null;
  use_vision_model?: boolean | null;
  keep_screenshots?: boolean | null;
  allowlist?: string[] | null;
  denylist?: string[] | null;
}

export interface LocalAiSettingsUpdate {
  runtime_enabled?: boolean | null;
  /**
   * MVP opt-in marker. Bootstrap hard-overrides status to "disabled" when
   * this is `false`, regardless of `runtime_enabled`. The unified AI panel
   * toggle flips this in tandem with `runtime_enabled` so a single click
   * actually turns local AI on — without it, the daemon spawns but
   * bootstrap immediately forces status back to disabled (cloud fallback).
   */
  opt_in_confirmed?: boolean | null;
  provider?: string | null;
  base_url?: string | null;
  model_id?: string | null;
  chat_model_id?: string | null;
  usage_embeddings?: boolean | null;
  usage_heartbeat?: boolean | null;
  usage_learning_reflection?: boolean | null;
  usage_subconscious?: boolean | null;
}

export interface RuntimeFlags {
  browser_allow_all: boolean;
  log_prompts: boolean;
}

export interface AIPreview {
  soul: {
    raw: string;
    name: string;
    description: string;
    personalityPreview: string[];
    safetyRulesPreview: string[];
    loadedAt: number;
  };
  tools: {
    raw: string;
    totalTools: number;
    activeSkills: number;
    skillsPreview: string[];
    loadedAt: number;
  };
  metadata: {
    loadedAt: number;
    loadingDuration: number;
    hasFallbacks: boolean;
    sources: { soul: string; tools: string };
    errors: string[];
  };
}

export async function openhumanGetConfig(): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({ method: CORE_RPC_METHODS.configGet });
}

/**
 * Safe client-facing config slice. Never contains the raw api_key — only
 * `api_key_set` indicates whether a custom backend key is stored. See
 * `config.get_client_config` in `src/openhuman/config/schemas.rs`.
 */
export interface ClientConfig {
  /** OpenHuman product backend URL (auth/billing/voice). */
  api_url: string | null;
  /**
   * Custom OpenAI-compatible LLM endpoint. Legacy field, retained for
   * back-compat — the new AI settings panel reads/writes
   * `cloud_providers` + `*_provider` fields instead.
   */
  inference_url: string | null;
  default_model: string | null;
  app_version: string;
  api_key_set: boolean;
  /** Legacy per-task-hint model overrides (deprecated; will be removed). */
  model_routes: ModelRoute[];
  /** Configured cloud providers (no API keys — those live in auth-profiles.json). */
  cloud_providers: CloudProviderCreds[];
  /** Id of the `cloud_providers` entry resolved by the `"cloud"` sentinel. */
  primary_cloud: string | null;
  /** Per-workload provider strings (e.g. `"cloud"`, `"ollama:llama3.1:8b"`, `"openai:gpt-4o"`). */
  chat_provider: string | null;
  reasoning_provider: string | null;
  agentic_provider: string | null;
  coding_provider: string | null;
  memory_provider: string | null;
  embeddings_provider: string | null;
  heartbeat_provider: string | null;
  learning_provider: string | null;
  subconscious_provider: string | null;
}

export async function openhumanGetClientConfig(): Promise<CommandResponse<ClientConfig>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ClientConfig>>({
    method: 'openhuman.inference_get_client_config',
  });
}

export async function openhumanUpdateModelSettings(
  update: ModelSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: 'openhuman.inference_update_model_settings',
    params: update,
  });
}

export async function openhumanUpdateMemorySettings(
  update: MemorySettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: CORE_RPC_METHODS.configUpdateMemorySettings,
    params: update,
  });
}

export async function openhumanUpdateRuntimeSettings(
  update: RuntimeSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: CORE_RPC_METHODS.configUpdateRuntimeSettings,
    params: update,
  });
}

export async function openhumanUpdateBrowserSettings(
  update: BrowserSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: CORE_RPC_METHODS.configUpdateBrowserSettings,
    params: update,
  });
}

export async function openhumanUpdateScreenIntelligenceSettings(
  update: ScreenIntelligenceSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: CORE_RPC_METHODS.configUpdateScreenIntelligenceSettings,
    params: update,
  });
}

// ── Agent access mode (autonomy / filesystem permissions) ───────────────────

export type AutonomyLevel = 'readonly' | 'supervised' | 'full';
export type TrustedAccess = 'read' | 'readwrite';

export interface TrustedRoot {
  path: string;
  access: TrustedAccess;
}

/** The full [autonomy] block as returned by config_get_autonomy_settings. */
export interface AutonomySettings {
  level: AutonomyLevel;
  workspace_only: boolean;
  allowed_commands: string[];
  forbidden_paths: string[];
  trusted_roots: TrustedRoot[];
  allow_tool_install: boolean;
  max_actions_per_hour: number;
  /** "Always allow" allowlist — tool names the agent runs without a prompt. */
  auto_approve: string[];
}

/** Partial update — omitted fields are left unchanged. */
export interface AutonomySettingsUpdate {
  level?: AutonomyLevel;
  workspace_only?: boolean;
  allowed_commands?: string[];
  forbidden_paths?: string[];
  trusted_roots?: TrustedRoot[];
  allow_tool_install?: boolean;
  max_actions_per_hour?: number;
  /** Replaces the "Always allow" allowlist wholesale. */
  auto_approve?: string[];
}

export async function openhumanGetAutonomySettings(): Promise<CommandResponse<AutonomySettings>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AutonomySettings>>({
    method: CORE_RPC_METHODS.configGetAutonomySettings,
  });
}

export async function openhumanUpdateAutonomySettings(
  update: AutonomySettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: CORE_RPC_METHODS.configUpdateAutonomySettings,
    params: update,
  });
}

export async function openhumanUpdateLocalAiSettings(
  update: LocalAiSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: 'openhuman.inference_update_local_settings',
    params: update,
  });
}

export async function openhumanUpdateAnalyticsSettings(update: {
  enabled?: boolean;
}): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: CORE_RPC_METHODS.configUpdateAnalyticsSettings,
    params: update,
  });
}

export async function openhumanGetAnalyticsSettings(): Promise<
  CommandResponse<{ enabled: boolean }>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<{ enabled: boolean }>>({
    method: CORE_RPC_METHODS.configGetAnalyticsSettings,
  });
}

export async function openhumanUpdateMeetSettings(update: {
  auto_orchestrator_handoff?: boolean;
}): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: 'openhuman.config_update_meet_settings',
    params: update,
  });
}

export async function openhumanGetMeetSettings(): Promise<
  CommandResponse<{ auto_orchestrator_handoff: boolean }>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<{ auto_orchestrator_handoff: boolean }>>({
    method: 'openhuman.config_get_meet_settings',
  });
}

export type SearchEngineId = 'managed' | 'parallel' | 'brave';

export interface SearchSettingsUpdate {
  engine?: SearchEngineId;
  max_results?: number;
  timeout_secs?: number;
  /** Empty string clears the stored key. */
  parallel_api_key?: string;
  /** Empty string clears the stored key. */
  brave_api_key?: string;
  /**
   * Websites the assistant may open/read (web_fetch / curl). Exact hosts
   * match their subdomains; `"*"` allows all public sites; an empty list
   * blocks all web access.
   */
  allowed_domains?: string[];
  /**
   * "Allow all sites" toggle. true → allowlist becomes `["*"]`.
   * NOTE: `allow_all` is applied AFTER `allowed_domains` server-side, so when
   * both are sent in one patch `allow_all` wins (true → `["*"]`, false → the
   * `"*"` wildcard is dropped). Don't send both with conflicting intent.
   */
  allow_all?: boolean;
}

export interface SearchSettings {
  engine: SearchEngineId | string;
  effective_engine: SearchEngineId;
  max_results: number;
  timeout_secs: number;
  parallel_configured: boolean;
  brave_configured: boolean;
  /** Current allowed-websites host list (may contain `"*"`). */
  allowed_domains: string[];
  /** True when the allowlist contains the `"*"` wildcard. */
  allow_all: boolean;
}

export interface DiagramViewerSettings {
  enabled: boolean;
  source_url: string;
  refresh_interval_seconds: number;
}

export interface DashboardSettings {
  diagram_viewer: DiagramViewerSettings;
}

export async function openhumanGetDashboardSettings(): Promise<CommandResponse<DashboardSettings>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<DashboardSettings>>({
    method: CORE_RPC_METHODS.configGetDashboardSettings,
  });
}

export async function openhumanGetSearchSettings(): Promise<CommandResponse<SearchSettings>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<SearchSettings>>({
    method: CORE_RPC_METHODS.configGetSearchSettings,
  });
}

export async function openhumanUpdateSearchSettings(
  update: SearchSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: CORE_RPC_METHODS.configUpdateSearchSettings,
    params: update,
  });
}

export interface ComposioTriggerSettingsUpdate {
  triage_disabled?: boolean | null;
  triage_disabled_toolkits?: string[] | null;
}

export interface ComposioTriggerSettings {
  triage_disabled: boolean;
  triage_disabled_toolkits: string[];
}

export async function openhumanUpdateComposioTriggerSettings(
  update: ComposioTriggerSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  try {
    return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
      method: 'openhuman.config_update_composio_trigger_settings',
      params: update,
    });
  } catch (err) {
    if (tauriErrorMessage(err).includes('unknown method')) {
      // Stale core sidecar predates composio trigger settings (#1597).
      log(
        '[composio:rpc] graceful degradation: stale core lacks config_update_composio_trigger_settings (#1597)'
      );
      return { result: { config: {}, workspace_dir: '', config_path: '' }, logs: [] };
    }
    throw err;
  }
}

export async function openhumanGetComposioTriggerSettings(): Promise<
  CommandResponse<ComposioTriggerSettings>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  try {
    return await callCoreRpc<CommandResponse<ComposioTriggerSettings>>({
      method: 'openhuman.config_get_composio_trigger_settings',
    });
  } catch (err) {
    if (tauriErrorMessage(err).includes('unknown method')) {
      // Stale core sidecar predates composio trigger settings (#1597).
      log(
        '[composio:rpc] graceful degradation: stale core lacks config_get_composio_trigger_settings (#1597)'
      );
      return { result: { triage_disabled: false, triage_disabled_toolkits: [] }, logs: [] };
    }
    throw err;
  }
}

export async function openhumanGetRuntimeFlags(): Promise<CommandResponse<RuntimeFlags>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<RuntimeFlags>>({
    method: CORE_RPC_METHODS.configGetRuntimeFlags,
  });
}

export async function openhumanSetBrowserAllowAll(
  enabled: boolean
): Promise<CommandResponse<RuntimeFlags>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<RuntimeFlags>>({
    method: CORE_RPC_METHODS.configSetBrowserAllowAll,
    params: { enabled },
  });
}

export async function aiGetConfig(): Promise<AIPreview> {
  return {
    soul: {
      raw: '',
      name: 'OpenHuman',
      description: 'Agent',
      personalityPreview: [],
      safetyRulesPreview: [],
      loadedAt: Date.now(),
    },
    tools: { raw: '', totalTools: 0, activeSkills: 0, skillsPreview: [], loadedAt: Date.now() },
    metadata: {
      loadedAt: Date.now(),
      loadingDuration: 0,
      hasFallbacks: true,
      sources: { soul: 'frontend', tools: 'frontend' },
      errors: ['AI prompt preview has been moved out of the Tauri host.'],
    },
  };
}

export async function aiRefreshConfig(): Promise<AIPreview> {
  return aiGetConfig();
}
