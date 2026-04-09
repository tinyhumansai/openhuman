/**
 * Accessibility and Screen Intelligence commands.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { CommandResponse, isTauri } from './common';

export type AccessibilityPermissionState = 'granted' | 'denied' | 'unknown' | 'unsupported';
export type AccessibilityPermissionKind = 'screen_recording' | 'accessibility' | 'input_monitoring';

export interface AccessibilityPermissionStatus {
  screen_recording: AccessibilityPermissionState;
  accessibility: AccessibilityPermissionState;
  input_monitoring: AccessibilityPermissionState;
}

export interface AccessibilityFeatures {
  screen_monitoring: boolean;
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
  use_vision_model: boolean;
  keep_screenshots: boolean;
  allowlist: string[];
  denylist: string[];
}

export interface AccessibilityCoreProcessStatus {
  pid: number;
  started_at_ms: number;
}

export interface AccessibilityStatus {
  platform_supported: boolean;
  permissions: AccessibilityPermissionStatus;
  features: AccessibilityFeatures;
  session: AccessibilitySessionStatus;
  config: AccessibilityConfig;
  denylist: string[];
  is_context_blocked: boolean;
  /** Absolute path of the core binary; macOS TCC applies to this executable. */
  permission_check_process_path?: string | null;
  /** Identity of the core process currently serving RPC requests. */
  core_process?: AccessibilityCoreProcessStatus | null;
}

export interface AccessibilityStartSessionParams {
  consent: boolean;
  ttl_secs?: number;
  screen_monitoring?: boolean;
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
