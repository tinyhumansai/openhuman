/**
 * Tauri Commands
 *
 * Helper functions for invoking Tauri commands from the frontend.
 */
import { isTauri as coreIsTauri, invoke } from '@tauri-apps/api/core';

// Check if we're running in Tauri
export const isTauri = (): boolean => {
  // Tauri v2: prefer the official runtime check over window globals.
  return coreIsTauri();
};

/**
 * Exchange a login token for a session token
 */
export async function exchangeToken(
  backendUrl: string,
  token: string
): Promise<{ sessionToken: string; user: object }> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }

  return await invoke('exchange_token', { backendUrl, token });
}

/**
 * Get the current authentication state from Rust
 */
export async function getAuthState(): Promise<{ is_authenticated: boolean; user: object | null }> {
  if (!isTauri()) {
    return { is_authenticated: false, user: null };
  }

  return await invoke('get_auth_state');
}

/**
 * Get the session token from secure storage
 */
export async function getSessionToken(): Promise<string | null> {
  if (!isTauri()) {
    return null;
  }

  return await invoke('get_session_token');
}

/**
 * Logout and clear session
 */
export async function logout(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await invoke('logout');
}

/**
 * Store session in secure storage
 */
export async function storeSession(token: string, user: object): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await invoke('store_session', { token, user });
}

/**
 * Show the main window
 */
export async function showWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await invoke('show_window');
}

/**
 * Hide the main window
 */
export async function hideWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await invoke('hide_window');
}

/**
 * Toggle window visibility
 */
export async function toggleWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await invoke('toggle_window');
}

/**
 * Check if window is visible
 */
export async function isWindowVisible(): Promise<boolean> {
  if (!isTauri()) {
    return true; // In browser, window is always visible
  }

  return await invoke('is_window_visible');
}

/**
 * Minimize the window
 */
export async function minimizeWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await invoke('minimize_window');
}

/**
 * Maximize or unmaximize the window
 */
export async function maximizeWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await invoke('maximize_window');
}

/**
 * Close the window (minimizes to tray on macOS)
 */
export async function closeWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await invoke('close_window');
}

/**
 * Set the window title
 */
export async function setWindowTitle(title: string): Promise<void> {
  if (!isTauri()) {
    document.title = title;
    return;
  }

  await invoke('set_window_title', { title });
}

// --- Alphahuman Commands ---

export type DoctorSeverity = 'Ok' | 'Warn' | 'Error';
export type ModelProbeOutcome = 'Ok' | 'Skipped' | 'AuthOrAccess' | 'Error';
export type IntegrationStatus = 'Available' | 'Active' | 'ComingSoon';
export type IntegrationCategory =
  | 'Chat'
  | 'AiModel'
  | 'Productivity'
  | 'MusicAudio'
  | 'SmartHome'
  | 'ToolsAutomation'
  | 'MediaCreative'
  | 'Social'
  | 'Platform';
export type ModelRefreshSource = 'Live' | 'CacheFresh' | 'CacheStaleFallback';
export type ServiceState = 'Running' | 'Stopped' | 'NotInstalled' | { Unknown: string };
export type HardwareTransport = 'Native' | 'Serial' | 'Probe' | 'None';

export interface CommandResponse<T> {
  result: T;
  logs: string[];
}

export interface SkillSnapshot {
  skill_id: string;
  name: string;
  status: unknown;
  tools: Array<{
    name: string;
    description: string;
    input_schema?: unknown;
  }>;
  error?: string | null;
  state?: Record<string, unknown>;
}

export interface DoctorReport {
  items: { severity: DoctorSeverity; category: string; message: string }[];
  summary: { ok: number; warnings: number; errors: number };
}

export interface ModelProbeReport {
  entries: { provider: string; outcome: ModelProbeOutcome; message?: string | null }[];
  summary: { ok: number; skipped: number; auth_or_access: number; errors: number };
}

export interface IntegrationInfo {
  name: string;
  description: string;
  category: IntegrationCategory;
  status: IntegrationStatus;
  setup_hints: string[];
}

export interface ModelRefreshResult {
  provider: string;
  models: string[];
  source: ModelRefreshSource;
  cache_age_secs?: number | null;
  warnings: string[];
}

export interface MigrationStats {
  from_sqlite: number;
  from_markdown: number;
  imported: number;
  skipped_unchanged: number;
  renamed_conflicts: number;
}

export interface MigrationReport {
  source_workspace: string;
  target_workspace: string;
  dry_run: boolean;
  stats: MigrationStats;
  warnings: string[];
}

export interface DiscoveredDevice {
  name: string;
  detail?: string | null;
  device_path?: string | null;
  transport: HardwareTransport;
}

export interface HardwareIntrospect {
  path: string;
  vid?: number | null;
  pid?: number | null;
  board_name?: string | null;
  architecture?: string | null;
  memory_map_note: string;
}

export interface ServiceStatus {
  state: ServiceState;
  unit_path?: string | null;
  label: string;
  details?: string | null;
}

export interface ConfigSnapshot {
  config: Record<string, unknown>;
  workspace_dir: string;
  config_path: string;
}

export interface ModelSettingsUpdate {
  api_key?: string | null;
  api_url?: string | null;
  default_provider?: string | null;
  default_model?: string | null;
  default_temperature?: number | null;
}

export interface MemorySettingsUpdate {
  backend?: string | null;
  auto_save?: boolean | null;
  embedding_provider?: string | null;
  embedding_model?: string | null;
  embedding_dimensions?: number | null;
}

export interface GatewaySettingsUpdate {
  host?: string | null;
  port?: number | null;
  require_pairing?: boolean | null;
  allow_public_bind?: boolean | null;
}

export interface RuntimeSettingsUpdate {
  kind?: string | null;
  reasoning_enabled?: boolean | null;
}

export interface BrowserSettingsUpdate {
  enabled?: boolean | null;
}

export interface RuntimeFlags {
  browser_allow_all: boolean;
  log_prompts: boolean;
}

export interface TunnelConfig {
  provider: string;
  cloudflare?: { token: string } | null;
  tailscale?: { funnel?: boolean; hostname?: string | null } | null;
  ngrok?: { auth_token: string; domain?: string | null } | null;
  custom?: { start_command: string; health_url?: string | null; url_pattern?: string | null } | null;
}

export async function alphahumanGetConfig(): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_get_config');
}

export async function alphahumanUpdateModelSettings(
  update: ModelSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_update_model_settings', { update });
}

export async function alphahumanUpdateMemorySettings(
  update: MemorySettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_update_memory_settings', { update });
}

export async function alphahumanUpdateGatewaySettings(
  update: GatewaySettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_update_gateway_settings', { update });
}

export async function alphahumanUpdateTunnelSettings(
  tunnel: TunnelConfig
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_update_tunnel_settings', { tunnel });
}

export async function alphahumanUpdateRuntimeSettings(
  update: RuntimeSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_update_runtime_settings', { update });
}

export async function alphahumanUpdateBrowserSettings(
  update: BrowserSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_update_browser_settings', { update });
}

export async function alphahumanGetRuntimeFlags(): Promise<CommandResponse<RuntimeFlags>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_get_runtime_flags');
}

export async function alphahumanSetBrowserAllowAll(
  enabled: boolean
): Promise<CommandResponse<RuntimeFlags>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_set_browser_allow_all', { enabled });
}

export async function alphahumanAgentChat(
  message: string,
  providerOverride?: string,
  modelOverride?: string,
  temperature?: number
): Promise<CommandResponse<string>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_agent_chat', {
    message,
    providerOverride,
    modelOverride,
    temperature,
  });
}

export async function alphahumanEncryptSecret(
  plaintext: string
): Promise<CommandResponse<string>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_encrypt_secret', { plaintext });
}

export async function alphahumanDecryptSecret(
  ciphertext: string
): Promise<CommandResponse<string>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_decrypt_secret', { ciphertext });
}

export async function alphahumanDoctorReport(): Promise<CommandResponse<DoctorReport>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_doctor_report');
}

export async function alphahumanDoctorModels(
  providerOverride?: string,
  useCache = true
): Promise<CommandResponse<ModelProbeReport>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_doctor_models', {
    providerOverride,
    useCache,
  });
}

export async function alphahumanListIntegrations(): Promise<CommandResponse<IntegrationInfo[]>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_list_integrations');
}

export async function alphahumanGetIntegrationInfo(
  name: string
): Promise<CommandResponse<IntegrationInfo>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_get_integration_info', { name });
}

export async function alphahumanModelsRefresh(
  providerOverride?: string,
  force = false
): Promise<CommandResponse<ModelRefreshResult>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_models_refresh', { providerOverride, force });
}

export async function alphahumanMigrateOpenclaw(
  sourceWorkspace?: string,
  dryRun = true
): Promise<CommandResponse<MigrationReport>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_migrate_openclaw', {
    sourceWorkspace,
    dryRun,
  });
}

export async function alphahumanHardwareDiscover(): Promise<CommandResponse<DiscoveredDevice[]>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_hardware_discover');
}

export async function alphahumanHardwareIntrospect(
  path: string
): Promise<CommandResponse<HardwareIntrospect>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_hardware_introspect', { path });
}

export async function alphahumanServiceInstall(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_service_install');
}

export async function alphahumanServiceStart(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_service_start');
}

export async function alphahumanServiceStop(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_service_stop');
}

export async function alphahumanServiceStatus(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_service_status');
}

export async function alphahumanServiceUninstall(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('alphahuman_service_uninstall');
}

export async function runtimeListSkills(): Promise<SkillSnapshot[]> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('runtime_list_skills');
}

export async function runtimeIsSkillEnabled(skillId: string): Promise<boolean> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await invoke('runtime_is_skill_enabled', { skill_id: skillId });
}

export async function runtimeEnableSkill(skillId: string): Promise<void> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  await invoke('runtime_enable_skill', { skill_id: skillId });
}

export async function runtimeDisableSkill(skillId: string): Promise<void> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  await invoke('runtime_disable_skill', { skill_id: skillId });
}
