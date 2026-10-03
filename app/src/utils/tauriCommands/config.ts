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
  browser_billing_route?: 'direct_openrouter' | 'hosted' | 'unavailable';
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
  /** USD per 1M input tokens. `0`/absent ⇒ unknown. Pre-filled for known
   *  vendor models from the Rust pricing catalog (`cost/catalog.rs`). */
  cost_per_1m_input?: number;
  /** USD per 1M cached-prefix input tokens. `0`/absent ⇒ unknown. */
  cost_per_1m_cached_input?: number;
  cost_per_1m_output: number;
  /** Max context window in tokens. `0`/absent ⇒ unknown. Pre-filled for known
   *  vendor models from the Rust pricing catalog. Used to budget prompts and
   *  trigger compaction — providers differ widely (128K–1M+). */
  context_window?: number;
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
  vision_provider?: string | null;
  memory_provider?: string | null;
  embeddings_provider?: string | null;
}

export interface RuntimeSettingsUpdate {
  kind?: string | null;
  reasoning_enabled?: boolean | null;
  /** Default thinking level for agent turns; `''` clears it to the provider default. */
  reasoning_effort?: string | null;
}

export interface BrowserSettingsUpdate {
  enabled?: boolean | null;
  backend?: 'tinycomputer' | null;
  headless?: boolean;
  viewport_width?: number;
  viewport_height?: number;
  chrome_path?: string | null;
  profile_mode?: 'fresh' | 'persistent';
  profile_path?: string | null;
  download_dir?: string | null;
  max_task_steps?: number;
  task_timeout_secs?: number;
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
  /**
   * Bearer credential for OpenAI-compatible local runtimes that require a key
   * (e.g. OMLX). Stored in `config.local_ai.api_key` and sent as a Bearer token
   * on inference. Keyless runtimes (Ollama / LM Studio) omit this.
   */
  api_key?: string | null;
  model_id?: string | null;
  chat_model_id?: string | null;
  usage_embeddings?: boolean | null;
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
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({ method: CORE_RPC_METHODS.configGet });
}

/**
 * Safe client-facing config slice. Never contains the raw api_key — only
 * `api_key_set` indicates whether a custom backend key is stored. See
 * `config.get_client_config` in `crates/openhuman-core/src/config/schemas.rs`.
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
  /**
   * The composer's thinking-level default (`runtime.reasoning_effort`):
   * `none` | `minimal` | `low` | `medium` | `high` | `xhigh`, or null when the
   * provider decides. Absent on cores that predate it.
   */
  reasoning_effort?: string | null;
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
  /**
   * #3767: authoritative, core-side per-tier flags — for each chat-mode tier
   * (`chat` = Quick mode, `reasoning` = Reasoning mode), true when that tier runs
   * on a non-managed provider the user funds themselves (a usable BYO key, local
   * runtime, or claude-code). The UI checks whichever tier the user has selected;
   * when true the "buy credits" prompt is suppressed for that mode. Optional for
   * back-compat with older snapshots.
   */
  credits_bypass?: { chat?: boolean; reasoning?: boolean };
  /** Per-workload provider strings (e.g. `"cloud"`, `"ollama:llama3.1:8b"`, `"openai:gpt-4o"`). */
  chat_provider: string | null;
  reasoning_provider: string | null;
  agentic_provider: string | null;
  coding_provider: string | null;
  vision_provider: string | null;
  memory_provider: string | null;
  embeddings_provider: string | null;
}

export async function openhumanGetClientConfig(): Promise<CommandResponse<ClientConfig>> {
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
  return await callCoreRpc<CommandResponse<ClaudeCodeStatus>>({
    method: 'openhuman.inference_claude_code_status',
  });
}

/**
 * Auth state for the Claude Code CLI provider — mirrors Rust
 * `claude_code::auth_status::AuthSource`. The `source` discriminator is
 * the snake_case Serde rename. `account_email` / `subscription_type` /
 * `expires_at` are best-effort: absent when the CLI's auth-status schema
 * drifts. `unknown` means we couldn't determine the state (binary missing,
 * spawn failed, or a CLI older than `auth status`) — it is NEVER signed-out.
 */
export type ClaudeCodeAuthStatus =
  | {
      source: 'subscription';
      account_email: string | null;
      subscription_type: string | null;
      expires_at: string | null;
      last_checked: number;
    }
  | { source: 'api_key_env'; last_checked: number }
  | { source: 'none'; last_checked: number }
  | { source: 'unknown'; reason: string | null; last_checked: number };

/**
 * Detect Claude Code CLI auth state via `claude auth status --json`
 * (cross-platform: abstracts the macOS Keychain vs. Linux/Windows file
 * stores), or `ANTHROPIC_API_KEY` env. Spawns the CLI — call on-demand /
 * Recheck, not on a tight loop.
 */
export async function openhumanClaudeCodeAuthStatus(): Promise<ClaudeCodeAuthStatus> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  // The core handler returns the value via `RpcOutcome::new(_, vec![])` with no
  // logs, which `into_cli_compatible_json` serializes as the BARE value (not a
  // `{ result, logs }` envelope). `callCoreRpc` returns the JSON-RPC `result`,
  // so this resolves directly to the AuthStatus — do NOT read `.result`.
  return await callCoreRpc<ClaudeCodeAuthStatus>({
    method: 'openhuman.inference_claude_code_auth_status',
  });
}

/**
 * Persisted Claude Code provider settings — mirrors Rust
 * `claude_code::settings::ClaudeCodeSettings`. `full_access=true` runs the
 * CLI with `--permission-mode bypassPermissions` + its full native toolset
 * (Bash/network/subagents); `false` (default) is the safer `acceptEdits`
 * posture (auto-apply file edits, gate the rest). On macOS the Seatbelt jail
 * still walls off `~/.openhuman` in either mode.
 */
export interface ClaudeCodeSettings {
  full_access: boolean;
}

/**
 * Read the persisted Claude Code full-access toggle. Bare value (no
 * `{ result, logs }` envelope) — see {@link openhumanClaudeCodeAuthStatus}.
 */
export async function openhumanClaudeCodeSettings(): Promise<ClaudeCodeSettings> {
  return await callCoreRpc<ClaudeCodeSettings>({
    method: 'openhuman.inference_claude_code_settings',
  });
}

/**
 * Persist the Claude Code full-access toggle. Returns the saved settings.
 * Takes effect on the next chat turn (the driver reads the file per-turn).
 */
export async function openhumanClaudeCodeSetFullAccess(
  enabled: boolean
): Promise<ClaudeCodeSettings> {
  return await callCoreRpc<ClaudeCodeSettings>({
    method: 'openhuman.inference_claude_code_set_full_access',
    params: { enabled },
  });
}

/**
 * Open the user's native terminal and run `claude auth login` inside it. The
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
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: 'openhuman.inference_update_model_settings',
    params: update,
  });
}

export async function openhumanUpdateRuntimeSettings(
  update: RuntimeSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: CORE_RPC_METHODS.configUpdateRuntimeSettings,
    params: update,
  });
}

/** TinyComputer's decision model family. */
export type DecisionModel = 'jev' | 'open_jev' | 'sage';

export interface ComputerSettings {
  decision_model: DecisionModel;
  sage_fast: boolean;
  planner_model?: string | null;
  rescue_model?: string | null;
  max_rescues?: number | null;
}

/** Partial update; an empty model string restores the module default. */
export interface ComputerSettingsUpdate {
  decision_model?: DecisionModel;
  sage_fast?: boolean;
  planner_model?: string;
  rescue_model?: string;
  max_rescues?: number;
}

export async function openhumanUpdateComputerSettings(
  update: ComputerSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: CORE_RPC_METHODS.configUpdateComputerSettings,
    params: update,
  });
}

export async function openhumanUpdateBrowserSettings(
  update: BrowserSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: CORE_RPC_METHODS.configUpdateBrowserSettings,
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
  /**
   * When true, the approval gate auto-approves ALL tool calls without
   * prompting — a blanket bypass, not just the `auto_approve` allowlist
   * above. Unlabelled origins are still denied by the gate regardless of
   * this flag; hard security blocks are unaffected.
   * Defaults to `false`.
   */
  auto_approve_all?: boolean;
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
  /** Blanket "auto-approve everything" bypass. See `AutonomySettings`. */
  auto_approve_all?: boolean;
}

export async function openhumanGetAutonomySettings(): Promise<CommandResponse<AutonomySettings>> {
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
 * - `files_dir` — the visible folder agent deliverables are written to
 *   (#5505); `default_files_dir` is `~/OpenHuman/projects/Files`, and
 *   `files_dir_source` says whether the user chose another one.
 */
export interface AgentPaths {
  action_dir: string;
  workspace_dir: string;
  projects_dir: string;
  action_dir_source: 'env' | 'override' | 'default';
  files_dir: string;
  default_files_dir: string;
  files_dir_source: 'override' | 'default';
}

export async function openhumanGetAgentPaths(): Promise<CommandResponse<AgentPaths>> {
  return await callCoreRpc<CommandResponse<AgentPaths>>({
    method: CORE_RPC_METHODS.configGetAgentPaths,
  });
}

/**
 * Partial update for the agent's editable filesystem roots (#3240, #5505).
 * An empty string reverts a field to its default; an omitted field is left
 * unchanged.
 */
export interface AgentPathsUpdate {
  action_dir?: string;
  files_dir?: string;
}

export async function openhumanUpdateAgentPaths(
  update: AgentPathsUpdate
): Promise<CommandResponse<AgentPaths>> {
  return await callCoreRpc<CommandResponse<AgentPaths>>({
    method: CORE_RPC_METHODS.configUpdateAgentPaths,
    params: update,
  });
}

export async function openhumanUpdateAutonomySettings(
  update: AutonomySettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
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
  return await callCoreRpc<CommandResponse<SandboxSettings>>({
    method: CORE_RPC_METHODS.configGetSandboxSettings,
  });
}

export async function openhumanUpdateSandboxSettings(
  update: SandboxSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: CORE_RPC_METHODS.configUpdateSandboxSettings,
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
  /** How tool calls are spoken to the model (`auto` = native/JSON, the default). */
  tool_dispatcher: ToolDispatcher;
  /** True when OPENHUMAN_TOOL_DISPATCHER overrides the configured value. */
  tool_dispatcher_env_override: boolean;
}

/** Accepted `agent.tool_dispatcher` values. */
export type ToolDispatcher = 'auto' | 'native' | 'xml' | 'pformat' | 'python' | 'typescript';

/** Partial update — omitted fields are left unchanged. */
export interface AgentSettingsUpdate {
  agent_timeout_secs?: number;
  tool_dispatcher?: ToolDispatcher;
}

export async function openhumanGetAgentSettings(): Promise<CommandResponse<AgentSettings>> {
  return await callCoreRpc<CommandResponse<AgentSettings>>({
    method: CORE_RPC_METHODS.configGetAgentSettings,
  });
}

export async function openhumanUpdateAgentSettings(
  update: AgentSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: CORE_RPC_METHODS.configUpdateAgentSettings,
    params: update,
  });
}

export async function openhumanUpdateLocalAiSettings(
  update: LocalAiSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: 'openhuman.inference_update_local_settings',
    params: update,
  });
}

export async function openhumanUpdateAnalyticsSettings(update: {
  enabled?: boolean;
}): Promise<CommandResponse<ConfigSnapshot>> {
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: CORE_RPC_METHODS.configUpdateAnalyticsSettings,
    params: update,
  });
}

export async function openhumanGetAnalyticsSettings(): Promise<
  CommandResponse<{ enabled: boolean }>
> {
  return await callCoreRpc<CommandResponse<{ enabled: boolean }>>({
    method: CORE_RPC_METHODS.configGetAnalyticsSettings,
  });
}

/** Capability role a search provider can serve; each role is one agent tool. */
export type SearchRole = 'search' | 'answer' | 'contents';

/** How a provider is reached: billed through TinyHumans, or with the user's own key. */
export type SearchRoute = 'managed' | 'direct';

/** Why a provider is or is not serving right now. */
export type SearchProviderStatus =
  | 'ready'
  | 'disabled'
  | 'needs_key'
  | 'sign_in_required'
  | 'search_off';

/** How the core exposes search to the agent. */
export type SearchPresentation = 'roles' | 'all_tools' | 'router' | 'one_provider';

/** One search provider as reported by `config_get_search_settings`. */
export interface SearchProviderInfo {
  id: string;
  label: string;
  enabled: boolean;
  route: SearchRoute;
  /** Routes this provider supports. */
  routes: SearchRoute[];
  managed_available: boolean;
  key_configured: boolean;
  takes_key: boolean;
  usable: boolean;
  status: SearchProviderStatus;
  /** Roles this provider is able to serve. */
  roles: SearchRole[];
  docs_url?: string | null;
  /** Gemini only: a direct key unlocks Deep Research for `depth: "deep"`. */
  deep_research_available?: boolean;
  /** SearXNG only: the instance base URL. */
  base_url?: string | null;
}

export interface SearchSettings {
  enabled: boolean;
  presentation: SearchPresentation;
  presentation_provider?: string | null;
  max_results: number;
  timeout_secs: number;
  /** False for a local (signed-out) session: managed routes cannot be used. */
  managed_available: boolean;
  providers: SearchProviderInfo[];
  /** Configured (or default) provider order per role. */
  roles: Record<SearchRole, string[]>;
  /** Usable providers per role in serving order; `[]` means the role has no tool. */
  effective_roles: Record<SearchRole, string[]>;
  /** Current allowed-websites host list (may contain `"*"`). */
  allowed_domains: string[];
  /** True when the allowlist contains the `"*"` wildcard. */
  allow_all: boolean;
}

/** Per-provider patch. `api_key: ""` clears the stored key. */
export interface SearchProviderUpdate {
  enabled?: boolean;
  route?: SearchRoute;
  api_key?: string;
  /** SearXNG only. */
  base_url?: string;
}

export interface SearchSettingsUpdate {
  enabled?: boolean;
  providers?: Record<string, SearchProviderUpdate>;
  /** Provider order per role; `[]` restores the default order. */
  roles?: Partial<Record<SearchRole, string[]>>;
  presentation?: SearchPresentation;
  presentation_provider?: string;
  max_results?: number;
  timeout_secs?: number;
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

export interface DiagramViewerSettings {
  enabled: boolean;
  source_url: string;
  refresh_interval_seconds: number;
}

export interface DashboardSettings {
  diagram_viewer: DiagramViewerSettings;
}

export async function openhumanGetDashboardSettings(): Promise<CommandResponse<DashboardSettings>> {
  return await callCoreRpc<CommandResponse<DashboardSettings>>({
    method: CORE_RPC_METHODS.configGetDashboardSettings,
  });
}

export async function openhumanGetSearchSettings(): Promise<CommandResponse<SearchSettings>> {
  // Plain core RPC: works from the desktop shell and from a browser attached
  // to the core (`pnpm dev:app:web`), so no Tauri guard.
  return await callCoreRpc<CommandResponse<SearchSettings>>({
    method: CORE_RPC_METHODS.configGetSearchSettings,
  });
}

export async function openhumanUpdateSearchSettings(
  update: SearchSettingsUpdate
): Promise<CommandResponse<SearchSettings>> {
  // Plain core RPC: works from the desktop shell and from a browser attached
  // to the core (`pnpm dev:app:web`), so no Tauri guard.
  return await callCoreRpc<CommandResponse<SearchSettings>>({
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
  return await callCoreRpc<CommandResponse<RuntimeFlags>>({
    method: CORE_RPC_METHODS.configGetRuntimeFlags,
  });
}

export async function openhumanSetBrowserAllowAll(
  enabled: boolean
): Promise<CommandResponse<RuntimeFlags>> {
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
