/**
 * Config and settings commands.
 */
import { invoke } from '@tauri-apps/api/core';
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

/**
 * Per-model registry entry. Mirrors the Rust `ModelRegistryEntry`
 * (`config/schema/types.rs`). Carries the user-set `vision` flag that lets a
 * custom/BYOK model accept chat image attachments.
 */
export interface ModelRegistryEntry {
  id: string;
  provider: string;
  cost_per_1m_output: number;
  vision: boolean;
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
  /**
   * When present, REPLACES `config.model_registry` wholesale. Carries each
   * model's `vision` flag (Settings → Advanced LLM → custom model → "Supports
   * vision"). Send `[]` to clear; omit to leave untouched.
   */
  model_registry?: ModelRegistryEntry[] | null;
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
  /** Per-model registry carrying each model's `vision` flag. */
  model_registry: ModelRegistryEntry[];
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

/**
 * Status payload for the Claude Code CLI provider — mirrors Rust
 * `claude_code::types::CliStatus`. The `status` discriminator is the
 * snake_case Serde rename; `path` and `version` may be absent depending
 * on which variant fired.
 */
export type ClaudeCodeStatus =
  | { status: 'ok'; version: string; path: string }
  | { status: 'not_installed' }
  | { status: 'outdated'; version: string; min_required: string; path: string }
  | { status: 'unusable'; path: string; reason: string };

/**
 * Probe the local `claude` CLI binary (Claude Code CLI provider). Returns
 * install + version status; never throws on a missing binary — the
 * `not_installed` variant signals that case explicitly.
 */
export async function openhumanClaudeCodeStatus(): Promise<CommandResponse<ClaudeCodeStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ClaudeCodeStatus>>({
    method: 'openhuman.inference_claude_code_status',
  });
}

/**
 * Auth state for the Claude Code CLI provider — mirrors Rust
 * `claude_code::auth_status::AuthSource`. The `source` discriminator is
 * the snake_case Serde rename. `account_email` / `expires_at` are
 * best-effort: absent when the CLI's credentials schema drifts.
 */
export type ClaudeCodeAuthStatus =
  | {
      source: 'subscription';
      account_email: string | null;
      expires_at: string | null;
      last_checked: number;
    }
  | { source: 'api_key_env'; last_checked: number }
  | { source: 'none'; last_checked: number };

/**
 * Detect Claude Code CLI auth state (Pro/Max subscription via
 * `~/.claude/.credentials.json`, `ANTHROPIC_API_KEY` env, or none).
 * Pure FS — no CLI spawn, safe to call on a tight refresh loop.
 */
export async function openhumanClaudeCodeAuthStatus(): Promise<
  CommandResponse<ClaudeCodeAuthStatus>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ClaudeCodeAuthStatus>>({
    method: 'openhuman.inference_claude_code_auth_status',
  });
}

/**
 * Open the user's native terminal and run `claude login` inside it. The
 * CLI's OAuth flow is interactive, so we can't host it in-app — we
 * detach into a terminal window and let the user complete the flow
 * there, then click Recheck back in the settings card.
 *
 * Returns the name of the terminal emulator that was launched.
 */
export async function openhumanClaudeCodeLoginLaunch(): Promise<string> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke<string>('claude_code_login_launch');
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
  /** Require approval before an agent executes a task-board plan. */
  require_task_plan_approval?: boolean;
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
  require_task_plan_approval?: boolean;
}

export async function openhumanGetAutonomySettings(): Promise<CommandResponse<AutonomySettings>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AutonomySettings>>({
    method: CORE_RPC_METHODS.configGetAutonomySettings,
  });
}

/**
 * Agent filesystem roots returned by `config_get_agent_paths`. All three are
 * already-canonicalised path strings; the UI renders them verbatim instead of
 * hard-coding defaults like `~/OpenHuman/projects`.
 *
 * - `action_dir` — agent CWD for `shell` / `node_exec` / `npm_exec` / file
 *   writes. Defaults to `projects_dir`; overridable via `OPENHUMAN_ACTION_DIR`.
 * - `workspace_dir` — internal product state (memory / sessions / vault).
 *   Agent-blocked.
 * - `projects_dir` — default projects home; matches `action_dir` when no
 *   override is set.
 * - `action_dir_source` — where the effective `action_dir` came from:
 *   `'env'` (pinned by OPENHUMAN_ACTION_DIR — UI must disable editing),
 *   `'override'` (a persisted user choice), or `'default'`.
 */
export interface AgentPaths {
  action_dir: string;
  workspace_dir: string;
  projects_dir: string;
  action_dir_source: 'env' | 'override' | 'default';
}

export async function openhumanGetAgentPaths(): Promise<CommandResponse<AgentPaths>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AgentPaths>>({
    method: CORE_RPC_METHODS.configGetAgentPaths,
  });
}

/** Partial update for the agent's editable filesystem roots (issue #3240). */
export interface AgentPathsUpdate {
  action_dir?: string;
}

export async function openhumanUpdateAgentPaths(
  update: AgentPathsUpdate
): Promise<CommandResponse<AgentPaths>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AgentPaths>>({
    method: CORE_RPC_METHODS.configUpdateAgentPaths,
    params: update,
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

// ── Sandbox execution backend settings ───────────────────────────────────────

export type SandboxBackendId = 'auto' | 'docker' | 'landlock' | 'firejail' | 'bubblewrap' | 'none';

/** Current sandbox settings returned by config_get_sandbox_settings. */
export interface SandboxSettings {
  enabled: boolean;
  backend: SandboxBackendId;
  docker_image: string;
  docker_memory_limit_mb: number | null;
  docker_cpu_limit: number | null;
  docker_available: boolean;
  detected_backend: string;
  env_passthrough: string[];
}

/** Partial update — omitted fields are left unchanged. */
export interface SandboxSettingsUpdate {
  backend?: SandboxBackendId;
  enabled?: boolean;
  docker_image?: string;
  docker_memory_limit_mb?: number | null;
  docker_cpu_limit?: number | null;
  env_passthrough?: string[];
}

export async function openhumanGetSandboxSettings(): Promise<CommandResponse<SandboxSettings>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<SandboxSettings>>({
    method: CORE_RPC_METHODS.configGetSandboxSettings,
  });
}

export async function openhumanUpdateSandboxSettings(
  update: SandboxSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: CORE_RPC_METHODS.configUpdateSandboxSettings,
    params: update,
  });
}

// ── Memory sync schedule (#3302) ─────────────────────────────────────────────

/** Global memory-sync schedule returned by config_get_memory_sync_settings. */
export interface MemorySyncSettings {
  /** Stored value: null = use the default cadence, 0 = Manual only, n>0 = seconds. */
  sync_interval_secs: number | null;
  /** Resolved cadence to highlight in the UI (the default when unset; 0 for manual). */
  selected_secs: number;
  /** True when the user picked "Manual only" (stored value is 0). */
  is_manual: boolean;
  /** True when no explicit choice is stored (falls back to `default_secs`). */
  is_default: boolean;
  /** The effective default cadence (seconds) applied when unset (24h). */
  default_secs: number;
  /** Preset cadences (seconds) offered in the UI: 4h / 12h / 24h. */
  presets: number[];
}

/** Partial update — set `sync_interval_secs` to `null` to reset to default. */
export interface MemorySyncSettingsUpdate {
  /** null = default, 0 = Manual only, n>0 = sync every n seconds. */
  sync_interval_secs?: number | null;
}

export async function openhumanGetMemorySyncSettings(): Promise<
  CommandResponse<MemorySyncSettings>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<MemorySyncSettings>>({
    method: CORE_RPC_METHODS.configGetMemorySyncSettings,
  });
}

export async function openhumanUpdateMemorySyncSettings(
  update: MemorySyncSettingsUpdate
): Promise<CommandResponse<MemorySyncSettings>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<MemorySyncSettings>>({
    method: CORE_RPC_METHODS.configUpdateMemorySyncSettings,
    params: update,
  });
}

// ── Agent execution settings (action/tool timeout) ──────────────────────────

/** Agent execution settings as returned by config_get_agent_settings. */
export interface AgentSettings {
  /** Configured wall-clock timeout for a single tool/action, in seconds. */
  agent_timeout_secs: number;
  /** Runtime-effective timeout (may differ from configured when env-overridden). */
  effective_timeout_secs: number;
  /** True when OPENHUMAN_TOOL_TIMEOUT_SECS overrides the configured value. */
  env_override: boolean;
  /** Lowest accepted timeout (seconds). */
  min_timeout_secs: number;
  /** Highest accepted timeout (seconds). */
  max_timeout_secs: number;
}

/** Partial update — omitted fields are left unchanged. */
export interface AgentSettingsUpdate {
  agent_timeout_secs?: number;
}

export async function openhumanGetAgentSettings(): Promise<CommandResponse<AgentSettings>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AgentSettings>>({
    method: CORE_RPC_METHODS.configGetAgentSettings,
  });
}

export async function openhumanUpdateAgentSettings(
  update: AgentSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: CORE_RPC_METHODS.configUpdateAgentSettings,
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

export type SearchEngineId = 'disabled' | 'managed' | 'parallel' | 'brave' | 'querit';

export interface SearchSettingsUpdate {
  engine?: SearchEngineId;
  max_results?: number;
  timeout_secs?: number;
  /** Empty string clears the stored key. */
  parallel_api_key?: string;
  /** Empty string clears the stored key. */
  brave_api_key?: string;
  /** Empty string clears the stored key. */
  querit_api_key?: string;
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
  querit_configured: boolean;
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
