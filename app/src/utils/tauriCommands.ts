/**
 * Tauri Commands
 *
 * Helper functions for invoking Tauri commands from the frontend.
 */
import { isTauri as coreIsTauri, invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';

import { callCoreRpc } from '../services/coreRpcClient';

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

  const response = await callCoreRpc<{ result: { isAuthenticated: boolean; user: object | null } }>(
    { method: 'openhuman.auth.get_state' }
  );

  return { is_authenticated: response.result.isAuthenticated, user: response.result.user };
}

/**
 * Get the session token from secure storage
 */
export async function getSessionToken(): Promise<string | null> {
  if (!isTauri()) {
    return null;
  }

  const response = await callCoreRpc<{ result: { token: string | null } }>({
    method: 'openhuman.auth.get_session_token',
  });
  return response.result.token;
}

/**
 * Logout and clear session
 */
export async function logout(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await callCoreRpc({ method: 'openhuman.auth.clear_session' });
}

/**
 * Store session in secure storage
 */
export async function storeSession(token: string, user: object): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await callCoreRpc({ method: 'openhuman.auth.store_session', params: { token, user } });
}

/**
 * Show the main window
 */
export async function showWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  const window = getCurrentWindow();
  await window.show();
  await window.unminimize();
  await window.setFocus();
}

/**
 * Hide the main window
 */
export async function hideWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await getCurrentWindow().hide();
}

/**
 * Toggle window visibility
 */
export async function toggleWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  const window = getCurrentWindow();
  const visible = await window.isVisible();
  if (visible) {
    await window.hide();
    return;
  }
  await window.show();
  await window.unminimize();
  await window.setFocus();
}

/**
 * Check if window is visible
 */
export async function isWindowVisible(): Promise<boolean> {
  if (!isTauri()) {
    return true; // In browser, window is always visible
  }

  return await getCurrentWindow().isVisible();
}

/**
 * Minimize the window
 */
export async function minimizeWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await getCurrentWindow().minimize();
}

/**
 * Maximize or unmaximize the window
 */
export async function maximizeWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  const window = getCurrentWindow();
  await window.toggleMaximize();
}

/**
 * Close the window (minimizes to tray on macOS)
 */
export async function closeWindow(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  await getCurrentWindow().close();
}

/**
 * Set the window title
 */
export async function setWindowTitle(title: string): Promise<void> {
  if (!isTauri()) {
    document.title = title;
    return;
  }

  await getCurrentWindow().setTitle(title);
}

// --- Memory Commands ---

/**
 * Initialise the TinyHumans memory client in Rust with the user's JWT token
 * (sourced from `authSlice.token` in Redux). Call this after login and after
 * Redux Persist rehydration.
 */
export async function syncMemoryClientToken(token: string): Promise<void> {
  console.debug(
    '[memory] syncMemoryClientToken: entry (token_present=%s, is_tauri=%s)',
    !!token,
    isTauri()
  );
  if (!isTauri() || !token) {
    console.debug('[memory] syncMemoryClientToken: exit — skipped (not Tauri or empty token)');
    return;
  }
  try {
    console.debug('[memory] syncMemoryClientToken: payload → memory.init');
    await callCoreRpc<boolean>({ method: 'memory.init', params: { jwt_token: token } });
    console.info('[memory] syncMemoryClientToken: exit — ok');
  } catch (err) {
    console.warn('[memory] syncMemoryClientToken: exit — error:', err);
  }
}

export interface MemoryDebugDocument {
  documentId: string;
  namespace: string;
  title?: string;
  raw: unknown;
}

export async function memoryListDocuments(namespace?: string): Promise<unknown> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<unknown>({ method: 'memory.list_documents', params: { namespace } });
}

export async function memoryListNamespaces(): Promise<string[]> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<string[]>({ method: 'memory.list_namespaces' });
}

export async function memoryDeleteDocument(
  documentId: string,
  namespace: string
): Promise<unknown> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<unknown>({
    method: 'memory.delete_document',
    params: { document_id: documentId, namespace },
  });
}

export async function memoryQueryNamespace(
  namespace: string,
  query: string,
  maxChunks?: number
): Promise<string> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<string>({
    method: 'memory.query_namespace',
    params: { namespace, query, max_chunks: maxChunks },
  });
}

export async function memoryRecallNamespace(
  namespace: string,
  maxChunks?: number
): Promise<string | null> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<string | null>({
    method: 'memory.recall_namespace',
    params: { namespace, max_chunks: maxChunks },
  });
}

export async function aiListMemoryFiles(relativeDir = 'memory'): Promise<string[]> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<string[]>({
    method: 'ai.list_memory_files',
    params: { relative_dir: relativeDir },
  });
}

export async function aiReadMemoryFile(relativePath: string): Promise<string> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<string>({
    method: 'ai.read_memory_file',
    params: { relative_path: relativePath },
  });
}

export async function aiWriteMemoryFile(relativePath: string, content: string): Promise<void> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  await callCoreRpc<boolean>({
    method: 'ai.write_memory_file',
    params: { relative_path: relativePath, content },
  });
}

/**
 * Trigger a conscious loop run manually.
 * The loop recalls all skill memory, extracts actionable items via LLM,
 * and stores them in the `conscious` namespace.
 */
export async function consciousLoopRun(
  authToken: string,
  backendUrl: string,
  model?: string
): Promise<void> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  await invoke('conscious_loop_run', { authToken, backendUrl, model });
}

// --- OpenHuman Commands ---

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
export type ServiceState = 'Running' | 'Stopped' | 'NotInstalled' | { Unknown: string };

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
export type HardwareTransport = 'Native' | 'Serial' | 'Probe' | 'None';

export interface CommandResponse<T> {
  result: T;
  logs: string[];
}

export interface SkillSnapshot {
  skill_id: string;
  name: string;
  status: unknown;
  tools: Array<{ name: string; description: string; input_schema?: unknown }>;
  error?: string | null;
  state?: Record<string, unknown>;
}

export interface RuntimeDiscoveredSkill {
  id: string;
  name: string;
  runtime?: string;
  entry?: string;
  autoStart?: boolean;
  version?: string;
  ignoreInProduction?: boolean;
  description?: string;
  platforms?: string[];
  tickInterval?: number | null;
  setup?: {
    required?: boolean;
    label?: string;
    oauth?: { provider: string; scopes: string[]; apiBaseUrl: string };
  };
}

export interface RuntimeSkillOption {
  name: string;
  type: 'boolean' | 'text' | 'number' | 'select';
  label: string;
  description?: string | null;
  default?: string | number | boolean | null;
  options?: Array<{ label: string; value: string }> | null;
  value?: string | number | boolean | null;
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

export interface AgentServerStatus {
  running: boolean;
  url: string;
}

export interface DaemonHostConfig {
  show_tray: boolean;
}

export type AccessibilityPermissionState = 'granted' | 'denied' | 'unknown' | 'unsupported';
export type AccessibilityPermissionKind = 'screen_recording' | 'accessibility' | 'input_monitoring';

export interface AccessibilityPermissionStatus {
  screen_recording: AccessibilityPermissionState;
  accessibility: AccessibilityPermissionState;
  input_monitoring: AccessibilityPermissionState;
}

export interface AccessibilityFeatures {
  screen_monitoring: boolean;
  device_control: boolean;
  predictive_input: boolean;
}

export interface AccessibilitySessionStatus {
  active: boolean;
  started_at_ms: number | null;
  expires_at_ms: number | null;
  remaining_ms: number | null;
  ttl_secs: number;
  panic_hotkey: string;
  stop_reason: string | null;
  frames_in_memory: number;
  last_capture_at_ms: number | null;
  last_context: string | null;
  vision_enabled: boolean;
  vision_state: string;
  vision_queue_depth: number;
  last_vision_at_ms: number | null;
  last_vision_summary: string | null;
}

export interface AccessibilityConfig {
  enabled: boolean;
  capture_policy: string;
  policy_mode: 'all_except_blacklist' | 'whitelist_only' | string;
  baseline_fps: number;
  vision_enabled: boolean;
  session_ttl_secs: number;
  panic_stop_hotkey: string;
  autocomplete_enabled: boolean;
  allowlist: string[];
  denylist: string[];
}

export interface AccessibilityStatus {
  platform_supported: boolean;
  permissions: AccessibilityPermissionStatus;
  features: AccessibilityFeatures;
  session: AccessibilitySessionStatus;
  config: AccessibilityConfig;
  denylist: string[];
  is_context_blocked: boolean;
}

export interface AccessibilityStartSessionParams {
  consent: boolean;
  ttl_secs?: number;
  screen_monitoring?: boolean;
  device_control?: boolean;
  predictive_input?: boolean;
}

export interface AccessibilityStopSessionParams {
  reason?: string;
}

export interface AccessibilityCaptureFrame {
  captured_at_ms: number;
  reason: string;
  app_name: string | null;
  window_title: string | null;
  image_ref?: string | null;
}

export interface AccessibilityCaptureNowResult {
  accepted: boolean;
  frame: AccessibilityCaptureFrame | null;
}

export interface AccessibilityInputActionParams {
  action: string;
  x?: number;
  y?: number;
  button?: string;
  text?: string;
  key?: string;
  modifiers?: string[];
}

export interface AccessibilityInputActionResult {
  accepted: boolean;
  blocked: boolean;
  reason: string | null;
}

export interface AccessibilityAutocompleteSuggestion {
  value: string;
  confidence: number;
}

export interface AccessibilityAutocompleteSuggestParams {
  context?: string;
  max_results?: number;
}

export interface AccessibilityAutocompleteSuggestResult {
  suggestions: AccessibilityAutocompleteSuggestion[];
}

export interface AccessibilityAutocompleteCommitParams {
  suggestion: string;
}

export interface AccessibilityAutocompleteCommitResult {
  committed: boolean;
}

export interface AutocompleteSuggestion {
  value: string;
  confidence: number;
}

export interface AutocompleteStatus {
  platform_supported: boolean;
  enabled: boolean;
  running: boolean;
  phase: string;
  debounce_ms: number;
  model_id: string;
  app_name?: string | null;
  last_error?: string | null;
  updated_at_ms?: number | null;
  suggestion?: AutocompleteSuggestion | null;
}

export interface AutocompleteStartParams {
  debounce_ms?: number;
}

export interface AutocompleteStartResult {
  started: boolean;
}

export interface AutocompleteStopParams {
  reason?: string;
}

export interface AutocompleteStopResult {
  stopped: boolean;
}

export interface AutocompleteCurrentParams {
  context?: string;
}

export interface AutocompleteCurrentResult {
  app_name?: string | null;
  context: string;
  suggestion?: AutocompleteSuggestion | null;
}

export interface AutocompleteDebugFocusResult {
  app_name?: string | null;
  role?: string | null;
  context: string;
  selected_text?: string | null;
  raw_error?: string | null;
}

export interface AutocompleteAcceptParams {
  suggestion?: string;
}

export interface AutocompleteAcceptResult {
  accepted: boolean;
  applied: boolean;
  value?: string | null;
  reason?: string | null;
}

export interface AutocompleteSetStyleParams {
  enabled?: boolean;
  debounce_ms?: number;
  max_chars?: number;
  style_preset?: string;
  style_instructions?: string;
  style_examples?: string[];
  disabled_apps?: string[];
  accept_with_tab?: boolean;
}

export interface AutocompleteConfig {
  enabled: boolean;
  debounce_ms: number;
  max_chars: number;
  style_preset: string;
  style_instructions?: string | null;
  style_examples: string[];
  disabled_apps: string[];
  accept_with_tab: boolean;
}

export interface AutocompleteSetStyleResult {
  config: AutocompleteConfig;
}

export interface AccessibilityVisionSummary {
  id: string;
  captured_at_ms: number;
  app_name: string | null;
  window_title: string | null;
  ui_state: string;
  key_text: string;
  actionable_notes: string;
  confidence: number;
}

export interface AccessibilityVisionRecentResult {
  summaries: AccessibilityVisionSummary[];
}

export interface AccessibilityVisionFlushResult {
  accepted: boolean;
  summary: AccessibilityVisionSummary | null;
}

export interface CaptureTestContextInfo {
  app_name: string | null;
  window_title: string | null;
  bounds_x: number | null;
  bounds_y: number | null;
  bounds_width: number | null;
  bounds_height: number | null;
}

export interface CaptureTestResult {
  ok: boolean;
  capture_mode: string;
  context: CaptureTestContextInfo | null;
  image_ref: string | null;
  bytes_estimate: number | null;
  error: string | null;
  timing_ms: number;
}

export interface ConfigSnapshot {
  config: Record<string, unknown>;
  workspace_dir: string;
  config_path: string;
}

export interface ModelSettingsUpdate {
  api_key?: string | null;
  api_url?: string | null;
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
  allowlist?: string[] | null;
  denylist?: string[] | null;
}

export interface RuntimeFlags {
  browser_allow_all: boolean;
  log_prompts: boolean;
}

export const DEFAULT_WORKSPACE_ONBOARDING_FLAG = '.skip_onboarding';

export interface LocalAiStatus {
  state: string;
  model_id: string;
  chat_model_id: string;
  vision_model_id: string;
  embedding_model_id: string;
  stt_model_id: string;
  tts_voice_id: string;
  quantization: string;
  vision_state: string;
  embedding_state: string;
  stt_state: string;
  tts_state: string;
  provider: string;
  download_progress?: number | null;
  downloaded_bytes?: number | null;
  total_bytes?: number | null;
  download_speed_bps?: number | null;
  eta_seconds?: number | null;
  warning?: string | null;
  model_path?: string | null;
  active_backend: string;
  backend_reason?: string | null;
  last_latency_ms?: number | null;
  prompt_toks_per_sec?: number | null;
  gen_toks_per_sec?: number | null;
}

export interface LocalAiSuggestion {
  text: string;
  confidence: number;
}

export interface LocalAiAssetStatus {
  state: string;
  id: string;
  provider: string;
  path?: string | null;
  warning?: string | null;
}

export interface LocalAiAssetsStatus {
  chat: LocalAiAssetStatus;
  vision: LocalAiAssetStatus;
  embedding: LocalAiAssetStatus;
  stt: LocalAiAssetStatus;
  tts: LocalAiAssetStatus;
  quantization: string;
}

export interface LocalAiDownloadProgressItem {
  id: string;
  provider: string;
  state: string;
  progress?: number | null;
  downloaded_bytes?: number | null;
  total_bytes?: number | null;
  speed_bps?: number | null;
  eta_seconds?: number | null;
  warning?: string | null;
  path?: string | null;
}

export interface LocalAiDownloadsProgress {
  state: string;
  warning?: string | null;
  progress?: number | null;
  downloaded_bytes?: number | null;
  total_bytes?: number | null;
  speed_bps?: number | null;
  eta_seconds?: number | null;
  chat: LocalAiDownloadProgressItem;
  vision: LocalAiDownloadProgressItem;
  embedding: LocalAiDownloadProgressItem;
  stt: LocalAiDownloadProgressItem;
  tts: LocalAiDownloadProgressItem;
}

export interface LocalAiEmbeddingResult {
  model_id: string;
  dimensions: number;
  vectors: number[][];
}

export interface LocalAiSpeechResult {
  text: string;
  model_id: string;
}

export interface LocalAiTtsResult {
  output_path: string;
  voice_id: string;
}

export interface RuntimeSkillDataStats {
  exists: boolean;
  path: string;
  total_bytes: number;
  file_count: number;
}

export interface CoreCronScheduleCron {
  kind: 'cron';
  expr: string;
  tz?: string | null;
}

export interface CoreCronScheduleAt {
  kind: 'at';
  at: string;
}

export interface CoreCronScheduleEvery {
  kind: 'every';
  every_ms: number;
}

export type CoreCronSchedule = CoreCronScheduleCron | CoreCronScheduleAt | CoreCronScheduleEvery;

export interface CoreCronJob {
  id: string;
  expression: string;
  schedule: CoreCronSchedule;
  command: string;
  prompt?: string | null;
  name?: string | null;
  job_type: 'shell' | 'agent' | string;
  session_target: 'isolated' | 'main' | string;
  model?: string | null;
  enabled: boolean;
  delivery: { mode: string; channel?: string | null; to?: string | null; best_effort: boolean };
  delete_after_run: boolean;
  created_at: string;
  next_run: string;
  last_run?: string | null;
  last_status?: string | null;
  last_output?: string | null;
}

export interface CoreCronRun {
  id: number;
  job_id: string;
  started_at: string;
  finished_at: string;
  status: string;
  output?: string | null;
  duration_ms?: number | null;
}

function tauriErrorMessage(err: unknown): string {
  if (err instanceof Error && err.message) {
    return err.message;
  }
  if (typeof err === 'string') {
    return err;
  }
  if (err && typeof err === 'object') {
    const maybeMessage = (err as { message?: unknown }).message;
    if (typeof maybeMessage === 'string' && maybeMessage.trim().length > 0) {
      return maybeMessage;
    }
    const maybeError = (err as { error?: unknown }).error;
    if (typeof maybeError === 'string' && maybeError.trim().length > 0) {
      return maybeError;
    }
  }
  return 'Unknown Tauri invoke error';
}

export interface TunnelConfig {
  provider: string;
  cloudflare?: { token: string } | null;
  tailscale?: { funnel?: boolean; hostname?: string | null } | null;
  ngrok?: { auth_token: string; domain?: string | null } | null;
  custom?: {
    start_command: string;
    health_url?: string | null;
    url_pattern?: string | null;
  } | null;
}

export async function openhumanGetConfig(): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({ method: 'openhuman.get_config' });
}

export async function openhumanUpdateModelSettings(
  update: ModelSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: 'openhuman.update_model_settings',
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
    method: 'openhuman.update_memory_settings',
    params: update,
  });
}

export async function openhumanUpdateTunnelSettings(
  tunnel: TunnelConfig
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: 'openhuman.update_tunnel_settings',
    params: tunnel,
  });
}

export async function openhumanUpdateRuntimeSettings(
  update: RuntimeSettingsUpdate
): Promise<CommandResponse<ConfigSnapshot>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: 'openhuman.update_runtime_settings',
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
    method: 'openhuman.update_browser_settings',
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
    method: 'openhuman.update_screen_intelligence_settings',
    params: update,
  });
}

export async function openhumanGetRuntimeFlags(): Promise<CommandResponse<RuntimeFlags>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<RuntimeFlags>>({
    method: 'openhuman.get_runtime_flags',
  });
}

export async function openhumanWorkspaceOnboardingFlagExists(
  flagName = DEFAULT_WORKSPACE_ONBOARDING_FLAG
): Promise<boolean> {
  if (!isTauri()) {
    return false;
  }
  return await callCoreRpc<boolean>({
    method: 'openhuman.workspace_onboarding_flag_exists',
    params: { flag_name: flagName },
  });
}

export async function openhumanWorkspaceOnboardingFlagSet(
  value: boolean,
  flagName = DEFAULT_WORKSPACE_ONBOARDING_FLAG
): Promise<boolean> {
  if (!isTauri()) {
    return false;
  }
  return await callCoreRpc<boolean>({
    method: 'openhuman.workspace_onboarding_flag_set',
    params: { flag_name: flagName, value },
  });
}

export async function openhumanSetBrowserAllowAll(
  enabled: boolean
): Promise<CommandResponse<RuntimeFlags>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<RuntimeFlags>>({
    method: 'openhuman.set_browser_allow_all',
    params: { enabled },
  });
}

export async function openhumanCronList(): Promise<CommandResponse<CoreCronJob[]>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<CoreCronJob[]>>({ method: 'openhuman.cron_list' });
}

export async function openhumanCronUpdate(
  jobId: string,
  patch: Record<string, unknown>
): Promise<CommandResponse<CoreCronJob>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<CoreCronJob>>({
    method: 'openhuman.cron_update',
    params: { job_id: jobId, patch },
  });
}

export async function openhumanCronRemove(
  jobId: string
): Promise<CommandResponse<{ job_id: string; removed: boolean }>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<{ job_id: string; removed: boolean }>>({
    method: 'openhuman.cron_remove',
    params: { job_id: jobId },
  });
}

export async function openhumanCronRun(
  jobId: string
): Promise<
  CommandResponse<{
    job_id: string;
    status: 'ok' | 'error' | string;
    duration_ms: number;
    output: string;
  }>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<
    CommandResponse<{
      job_id: string;
      status: 'ok' | 'error' | string;
      duration_ms: number;
      output: string;
    }>
  >({ method: 'openhuman.cron_run', params: { job_id: jobId } });
}

export async function openhumanCronRuns(
  jobId: string,
  limit = 20
): Promise<CommandResponse<CoreCronRun[]>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<CoreCronRun[]>>({
    method: 'openhuman.cron_runs',
    params: { job_id: jobId, limit },
  });
}

export async function openhumanAgentChat(
  message: string,
  modelOverride?: string,
  temperature?: number
): Promise<CommandResponse<string>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<string>>({
    method: 'openhuman.agent_chat',
    params: { message, model_override: modelOverride, temperature },
  });
}

export async function openhumanLocalAiStatus(): Promise<CommandResponse<LocalAiStatus>> {
  try {
    return await callCoreRpc<CommandResponse<LocalAiStatus>>({
      method: 'openhuman.local_ai_status',
    });
  } catch (err) {
    const message = tauriErrorMessage(err);
    if (message.includes('unknown method: openhuman.local_ai_status')) {
      throw new Error(
        'Local model runtime is unavailable in this core build. Restart app after updating to the latest build.'
      );
    }
    throw new Error(message);
  }
}

export async function openhumanLocalAiDownload(
  force?: boolean
): Promise<CommandResponse<LocalAiStatus>> {
  try {
    return await callCoreRpc<CommandResponse<LocalAiStatus>>({
      method: 'openhuman.local_ai_download',
      params: { force: force ?? false },
    });
  } catch (err) {
    const message = tauriErrorMessage(err);
    if (message.includes('unknown method: openhuman.local_ai_download')) {
      return await openhumanLocalAiStatus();
    }
    throw new Error(message);
  }
}

export async function openhumanLocalAiDownloadAllAssets(
  force?: boolean
): Promise<CommandResponse<LocalAiDownloadsProgress>> {
  return await callCoreRpc<CommandResponse<LocalAiDownloadsProgress>>({
    method: 'openhuman.local_ai_download_all_assets',
    params: { force: force ?? false },
  });
}

export async function openhumanLocalAiSummarize(
  text: string,
  maxTokens?: number
): Promise<CommandResponse<string>> {
  return await callCoreRpc<CommandResponse<string>>({
    method: 'openhuman.local_ai_summarize',
    params: { text, max_tokens: maxTokens },
  });
}

export async function openhumanLocalAiSuggestQuestions(
  context?: string,
  lines?: string[]
): Promise<CommandResponse<LocalAiSuggestion[]>> {
  return await callCoreRpc<CommandResponse<LocalAiSuggestion[]>>({
    method: 'openhuman.local_ai_suggest_questions',
    params: { context, lines },
  });
}

export async function openhumanLocalAiPrompt(
  prompt: string,
  maxTokens?: number,
  noThink?: boolean
): Promise<CommandResponse<string>> {
  return await callCoreRpc<CommandResponse<string>>({
    method: 'openhuman.local_ai_prompt',
    params: { prompt, max_tokens: maxTokens, no_think: noThink },
  });
}

export async function openhumanLocalAiVisionPrompt(
  prompt: string,
  imageRefs: string[],
  maxTokens?: number
): Promise<CommandResponse<string>> {
  return await callCoreRpc<CommandResponse<string>>({
    method: 'openhuman.local_ai_vision_prompt',
    params: { prompt, image_refs: imageRefs, max_tokens: maxTokens },
  });
}

export async function openhumanLocalAiEmbed(
  inputs: string[]
): Promise<CommandResponse<LocalAiEmbeddingResult>> {
  return await callCoreRpc<CommandResponse<LocalAiEmbeddingResult>>({
    method: 'openhuman.local_ai_embed',
    params: { inputs },
  });
}

export async function openhumanLocalAiTranscribe(
  audioPath: string
): Promise<CommandResponse<LocalAiSpeechResult>> {
  return await callCoreRpc<CommandResponse<LocalAiSpeechResult>>({
    method: 'openhuman.local_ai_transcribe',
    params: { audio_path: audioPath },
  });
}

export async function openhumanLocalAiTranscribeBytes(
  audioBytes: number[],
  extension?: string
): Promise<CommandResponse<LocalAiSpeechResult>> {
  return await callCoreRpc<CommandResponse<LocalAiSpeechResult>>({
    method: 'openhuman.local_ai_transcribe_bytes',
    params: { audio_bytes: audioBytes, extension },
  });
}

export async function openhumanLocalAiTts(
  text: string,
  outputPath?: string
): Promise<CommandResponse<LocalAiTtsResult>> {
  return await callCoreRpc<CommandResponse<LocalAiTtsResult>>({
    method: 'openhuman.local_ai_tts',
    params: { text, output_path: outputPath },
  });
}

export async function openhumanLocalAiAssetsStatus(): Promise<
  CommandResponse<LocalAiAssetsStatus>
> {
  return await callCoreRpc<CommandResponse<LocalAiAssetsStatus>>({
    method: 'openhuman.local_ai_assets_status',
  });
}

export async function openhumanLocalAiDownloadsProgress(): Promise<
  CommandResponse<LocalAiDownloadsProgress>
> {
  return await callCoreRpc<CommandResponse<LocalAiDownloadsProgress>>({
    method: 'openhuman.local_ai_downloads_progress',
  });
}

export async function openhumanLocalAiDownloadAsset(
  capability: 'chat' | 'vision' | 'embedding' | 'stt' | 'tts'
): Promise<CommandResponse<LocalAiAssetsStatus>> {
  return await callCoreRpc<CommandResponse<LocalAiAssetsStatus>>({
    method: 'openhuman.local_ai_download_asset',
    params: { capability },
  });
}

// --- Device profile & model tier presets ---

export interface DeviceProfileResult {
  total_ram_bytes: number;
  cpu_count: number;
  cpu_brand: string;
  os_name: string;
  os_version: string;
  has_gpu: boolean;
  gpu_description: string | null;
}

export interface ModelPresetResult {
  tier: string;
  label: string;
  description: string;
  chat_model_id: string;
  vision_model_id: string;
  embedding_model_id: string;
  quantization: string;
  min_ram_gb: number;
  approx_download_gb: number;
}

export interface PresetsResponse {
  presets: ModelPresetResult[];
  recommended_tier: string;
  current_tier: string;
  device: DeviceProfileResult;
}

export interface ApplyPresetResult {
  applied_tier: string;
  chat_model_id: string;
  vision_model_id: string;
  embedding_model_id: string;
  quantization: string;
}

export async function openhumanLocalAiDeviceProfile(): Promise<DeviceProfileResult> {
  return await callCoreRpc<DeviceProfileResult>({ method: 'openhuman.local_ai_device_profile' });
}

export async function openhumanLocalAiPresets(): Promise<PresetsResponse> {
  return await callCoreRpc<PresetsResponse>({ method: 'openhuman.local_ai_presets' });
}

export async function openhumanLocalAiApplyPreset(tier: string): Promise<ApplyPresetResult> {
  return await callCoreRpc<ApplyPresetResult>({
    method: 'openhuman.local_ai_apply_preset',
    params: { tier },
  });
}

export async function aiGetConfig(): Promise<AIPreview> {
  return {
    soul: {
      raw: '',
      name: 'OpenHuman',
      description: 'AI assistant',
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

export async function openhumanEncryptSecret(plaintext: string): Promise<CommandResponse<string>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<string>>({
    method: 'openhuman.encrypt_secret',
    params: { plaintext },
  });
}

export async function openhumanDecryptSecret(ciphertext: string): Promise<CommandResponse<string>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<string>>({
    method: 'openhuman.decrypt_secret',
    params: { ciphertext },
  });
}

export async function openhumanDoctorReport(): Promise<CommandResponse<DoctorReport>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<DoctorReport>>({ method: 'openhuman.doctor_report' });
}

export async function openhumanDoctorModels(
  useCache = true
): Promise<CommandResponse<ModelProbeReport>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ModelProbeReport>>({
    method: 'openhuman.doctor_models',
    params: { use_cache: useCache },
  });
}

export async function openhumanMigrateOpenclaw(
  sourceWorkspace?: string,
  dryRun = true
): Promise<CommandResponse<MigrationReport>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<MigrationReport>>({
    method: 'openhuman.migrate_openclaw',
    params: { source_workspace: sourceWorkspace, dry_run: dryRun },
  });
}

export async function openhumanHardwareDiscover(): Promise<CommandResponse<DiscoveredDevice[]>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<DiscoveredDevice[]>>({
    method: 'openhuman.hardware_discover',
  });
}

export async function openhumanHardwareIntrospect(
  path: string
): Promise<CommandResponse<HardwareIntrospect>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<HardwareIntrospect>>({
    method: 'openhuman.hardware_introspect',
    params: { path },
  });
}

export async function openhumanServiceInstall(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ServiceStatus>>({ method: 'openhuman.service_install' });
}

export async function openhumanServiceStart(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ServiceStatus>>({ method: 'openhuman.service_start' });
}

export async function openhumanServiceStop(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ServiceStatus>>({ method: 'openhuman.service_stop' });
}

export async function openhumanServiceStatus(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ServiceStatus>>({ method: 'openhuman.service_status' });
}

export async function openhumanServiceUninstall(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<ServiceStatus>>({
    method: 'openhuman.service_uninstall',
  });
}

export async function openhumanAgentServerStatus(): Promise<CommandResponse<AgentServerStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AgentServerStatus>>({
    method: 'openhuman.agent_server_status',
  });
}

export async function openhumanGetDaemonHostConfig(): Promise<CommandResponse<DaemonHostConfig>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<DaemonHostConfig>>({
    method: 'openhuman.service_daemon_host_get',
  });
}

export async function openhumanSetDaemonHostConfig(
  showTray: boolean
): Promise<CommandResponse<DaemonHostConfig>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<DaemonHostConfig>>({
    method: 'openhuman.service_daemon_host_set',
    params: { show_tray: showTray },
  });
}

export async function openhumanAccessibilityStatus(): Promise<
  CommandResponse<AccessibilityStatus>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AccessibilityStatus>>({
    method: 'openhuman.accessibility_status',
    serviceManaged: true,
  });
}

export async function openhumanAccessibilityRequestPermissions(): Promise<
  CommandResponse<AccessibilityPermissionStatus>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AccessibilityPermissionStatus>>({
    method: 'openhuman.accessibility_request_permissions',
    serviceManaged: true,
  });
}

export async function openhumanAccessibilityRequestPermission(
  permission: AccessibilityPermissionKind
): Promise<CommandResponse<AccessibilityPermissionStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AccessibilityPermissionStatus>>({
    method: 'openhuman.accessibility_request_permission',
    params: { permission },
    serviceManaged: true,
  });
}

export async function openhumanAccessibilityStartSession(
  params: AccessibilityStartSessionParams
): Promise<CommandResponse<AccessibilitySessionStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AccessibilitySessionStatus>>({
    method: 'openhuman.accessibility_start_session',
    params,
    serviceManaged: true,
  });
}

export async function openhumanAccessibilityStopSession(
  params?: AccessibilityStopSessionParams
): Promise<CommandResponse<AccessibilitySessionStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AccessibilitySessionStatus>>({
    method: 'openhuman.accessibility_stop_session',
    params: params ?? {},
    serviceManaged: true,
  });
}

export async function openhumanAccessibilityCaptureNow(): Promise<
  CommandResponse<AccessibilityCaptureNowResult>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AccessibilityCaptureNowResult>>({
    method: 'openhuman.accessibility_capture_now',
    serviceManaged: true,
  });
}

export async function openhumanAccessibilityInputAction(
  params: AccessibilityInputActionParams
): Promise<CommandResponse<AccessibilityInputActionResult>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AccessibilityInputActionResult>>({
    method: 'openhuman.accessibility_input_action',
    params,
    serviceManaged: true,
  });
}

export async function openhumanAccessibilityAutocompleteSuggest(
  params?: AccessibilityAutocompleteSuggestParams
): Promise<CommandResponse<AccessibilityAutocompleteSuggestResult>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AccessibilityAutocompleteSuggestResult>>({
    method: 'openhuman.accessibility_autocomplete_suggest',
    params: params ?? {},
    serviceManaged: true,
  });
}

export async function openhumanAccessibilityAutocompleteCommit(
  params: AccessibilityAutocompleteCommitParams
): Promise<CommandResponse<AccessibilityAutocompleteCommitResult>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AccessibilityAutocompleteCommitResult>>({
    method: 'openhuman.accessibility_autocomplete_commit',
    params,
    serviceManaged: true,
  });
}

export async function openhumanAccessibilityVisionRecent(
  limit?: number
): Promise<CommandResponse<AccessibilityVisionRecentResult>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AccessibilityVisionRecentResult>>({
    method: 'openhuman.accessibility_vision_recent',
    params: { limit },
    serviceManaged: true,
  });
}

export async function openhumanAccessibilityVisionFlush(): Promise<
  CommandResponse<AccessibilityVisionFlushResult>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AccessibilityVisionFlushResult>>({
    method: 'openhuman.accessibility_vision_flush',
    serviceManaged: true,
  });
}

export async function openhumanScreenIntelligenceCaptureTest(): Promise<
  CommandResponse<CaptureTestResult>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<CaptureTestResult>>({
    method: 'openhuman.screen_intelligence_capture_test',
    serviceManaged: true,
  });
}

export async function openhumanAutocompleteStatus(): Promise<CommandResponse<AutocompleteStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AutocompleteStatus>>({
    method: 'openhuman.autocomplete_status',
  });
}

export async function openhumanAutocompleteStart(
  params?: AutocompleteStartParams
): Promise<CommandResponse<AutocompleteStartResult>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AutocompleteStartResult>>({
    method: 'openhuman.autocomplete_start',
    params: params ?? {},
  });
}

export async function openhumanAutocompleteStop(
  params?: AutocompleteStopParams
): Promise<CommandResponse<AutocompleteStopResult>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AutocompleteStopResult>>({
    method: 'openhuman.autocomplete_stop',
    params: params ?? {},
  });
}

export async function openhumanAutocompleteCurrent(
  params?: AutocompleteCurrentParams
): Promise<CommandResponse<AutocompleteCurrentResult>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AutocompleteCurrentResult>>({
    method: 'openhuman.autocomplete_current',
    params: params ?? {},
  });
}

export async function openhumanAutocompleteDebugFocus(): Promise<
  CommandResponse<AutocompleteDebugFocusResult>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AutocompleteDebugFocusResult>>({
    method: 'openhuman.autocomplete_debug_focus',
  });
}

export async function openhumanAutocompleteAccept(
  params?: AutocompleteAcceptParams
): Promise<CommandResponse<AutocompleteAcceptResult>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AutocompleteAcceptResult>>({
    method: 'openhuman.autocomplete_accept',
    params: params ?? {},
  });
}

export async function openhumanAutocompleteSetStyle(
  params: AutocompleteSetStyleParams
): Promise<CommandResponse<AutocompleteSetStyleResult>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AutocompleteSetStyleResult>>({
    method: 'openhuman.autocomplete_set_style',
    params,
  });
}

export interface AcceptedCompletion {
  context: string;
  suggestion: string;
  app_name?: string | null;
  timestamp_ms: number;
}

export interface AutocompleteHistoryResult {
  entries: AcceptedCompletion[];
}

export interface AutocompleteClearHistoryResult {
  cleared: number;
}

export async function openhumanAutocompleteHistory(params?: {
  limit?: number;
}): Promise<CommandResponse<AutocompleteHistoryResult>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AutocompleteHistoryResult>>({
    method: 'openhuman.autocomplete_history',
    params: params ?? {},
  });
}

export async function openhumanAutocompleteClearHistory(): Promise<
  CommandResponse<AutocompleteClearHistoryResult>
> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AutocompleteClearHistoryResult>>({
    method: 'openhuman.autocomplete_clear_history',
  });
}

export async function runtimeListSkills(): Promise<SkillSnapshot[]> {
  return await callCoreRpc<SkillSnapshot[]>({ method: 'openhuman.skills_list' });
}

export async function runtimeDiscoverSkills(): Promise<RuntimeDiscoveredSkill[]> {
  return await callCoreRpc<RuntimeDiscoveredSkill[]>({ method: 'openhuman.skills_discover' });
}

export async function runtimeStartSkill(skillId: string): Promise<SkillSnapshot> {
  return await callCoreRpc<SkillSnapshot>({
    method: 'openhuman.skills_start',
    params: { skill_id: skillId },
  });
}

export async function runtimeStopSkill(skillId: string): Promise<void> {
  await callCoreRpc({ method: 'openhuman.skills_stop', params: { skill_id: skillId } });
}

export async function runtimeRpc<T = unknown>(
  skillId: string,
  method: string,
  params: Record<string, unknown> = {}
): Promise<T> {
  return await callCoreRpc<T>({
    method: 'openhuman.skills_rpc',
    params: { skill_id: skillId, method, params },
  });
}

export async function runtimeSkillDataRead(skillId: string, filename: string): Promise<string> {
  const result = await callCoreRpc<{ content: string }>({
    method: 'openhuman.skills_data_read',
    params: { skill_id: skillId, filename },
  });
  return result.content;
}

export async function runtimeSkillDataWrite(
  skillId: string,
  filename: string,
  content: string
): Promise<void> {
  await callCoreRpc({
    method: 'openhuman.skills_data_write',
    params: { skill_id: skillId, filename, content },
  });
}

export async function runtimeSkillDataDir(skillId: string): Promise<string> {
  const result = await callCoreRpc<{ path: string }>({
    method: 'openhuman.skills_data_dir',
    params: { skill_id: skillId },
  });
  return result.path;
}

export async function runtimeListSkillOptions(skillId: string): Promise<RuntimeSkillOption[]> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  const response = await runtimeRpc<{ options?: RuntimeSkillOption[] }>(
    skillId,
    'options/list',
    {}
  );
  return response.options ?? [];
}

export async function runtimeSetSkillOption(
  skillId: string,
  name: string,
  value: unknown
): Promise<void> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  await runtimeRpc(skillId, 'options/set', { name, value });
}

export async function runtimeIsSkillEnabled(skillId: string): Promise<boolean> {
  const result = await callCoreRpc<{ enabled: boolean }>({
    method: 'openhuman.skills_is_enabled',
    params: { skill_id: skillId },
  });
  return result.enabled;
}

export async function runtimeEnableSkill(skillId: string): Promise<void> {
  await callCoreRpc({ method: 'openhuman.skills_enable', params: { skill_id: skillId } });
}

export async function runtimeDisableSkill(skillId: string): Promise<void> {
  await callCoreRpc({ method: 'openhuman.skills_disable', params: { skill_id: skillId } });
}

export async function runtimeSkillDataStats(skillId: string): Promise<RuntimeSkillDataStats> {
  return await callCoreRpc<RuntimeSkillDataStats>({
    method: 'openhuman.skills_status',
    params: { skill_id: skillId },
  });
}
