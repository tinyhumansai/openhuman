/**
 * Config and settings commands.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { CORE_RPC_METHODS } from '../../services/rpcMethods';
import { CommandResponse, isTauri } from './common';

export interface ConfigSnapshot {
  config: Record<string, unknown>;
  workspace_dir: string;
  config_path: string;
}

export interface ModelRoute {
  hint: string;
  model: string;
}

/** Cloud provider type discriminator. Lowercase JSON wire format. */
export type CloudProviderType = 'openhuman' | 'openai' | 'anthropic' | 'openrouter' | 'custom';

/**
 * Endpoint config for one cloud LLM provider. API keys are NOT carried on
 * this struct — they live in `auth-profiles.json` via the AuthService
 * (set/cleared through the `auth_*` RPCs).
 */
export interface CloudProviderCreds {
  /** Opaque stable id, e.g. `"p_openai_a8c3f"`. Never shown in UI. */
  id: string;
  type: CloudProviderType;
  endpoint: string;
  default_model: string;
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
   */
  cloud_providers?: CloudProviderCreds[] | null;
  /** Id of the `cloud_providers` entry used when a workload routes to "cloud". */
  primary_cloud?: string | null;
  /** Per-workload provider strings — see Rust `providers::factory` grammar. */
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
    method: 'openhuman.config_get_client_config',
  });
}

export async function openhumanUpdateModelSettings(
  update: ModelSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: CORE_RPC_METHODS.configUpdateModelSettings,
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

export async function openhumanUpdateLocalAiSettings(
  update: LocalAiSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: 'openhuman.config_update_local_ai_settings',
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
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: 'openhuman.config_update_composio_trigger_settings',
    params: update,
  });
}

export async function openhumanGetComposioTriggerSettings(): Promise<
  CommandResponse<ComposioTriggerSettings>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ComposioTriggerSettings>>({
    method: 'openhuman.config_get_composio_trigger_settings',
  });
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
