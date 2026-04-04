/**
 * Type definitions for the skills system.
 * Mirrors the Python types in dev/types/setup_types.py and dev/types/skill_types.py.
 */

// ---------------------------------------------------------------------------
// Skill Manifest (from manifest.json)
// ---------------------------------------------------------------------------

export type SkillPlatform = "windows" | "macos" | "linux";

/** Unified registry skill type discriminant. */
export type SkillType = 'openhuman' | 'openclaw';

export interface SkillManifest {
  id: string;
  name: string;
  version: string;
  description: string;
  runtime: "quickjs";
  entry?: string;
  tick_interval?: number;
  env?: string[];
  dependencies?: string[];
  setup?: {
    required: boolean;
    label?: string;
    /** Legacy OAuth configuration. Prefer `auth` for new skills. */
    oauth?: {
      provider: string;
      scopes: string[];
      apiBaseUrl: string;
    };
    /** Advanced auth configuration with multiple auth modes. */
    auth?: SkillAuthConfig;
  };
  /** Platform filter. When present, only listed platforms load this skill.
   *  When absent or empty, the skill is available on all platforms. */
  platforms?: SkillPlatform[];
  /** When true, skill is hidden in production builds. */
  ignoreInProduction?: boolean;
  /** Unified registry type: "openhuman" (QuickJS) or "openclaw" (skill.md/TOML). */
  skill_type?: SkillType;
}

// ---------------------------------------------------------------------------
// Setup Flow Types
// ---------------------------------------------------------------------------

export interface SetupFieldOption {
  label: string;
  value: string;
}

export interface SetupField {
  name: string;
  type: "text" | "number" | "password" | "select" | "multiselect" | "boolean" | "textarea";
  label: string;
  description?: string | null;
  required: boolean;
  default?: string | number | boolean | string[] | null;
  placeholder?: string | null;
  options?: SetupFieldOption[] | null;
}

export interface SetupStep {
  id: string;
  title: string;
  description?: string | null;
  fields: SetupField[];
}

export interface SetupFieldError {
  field: string;
  message: string;
}

export interface SetupResult {
  status: "next" | "error" | "complete";
  nextStep?: SetupStep | null;
  errors?: SetupFieldError[] | null;
  message?: string | null;
}

// ---------------------------------------------------------------------------
// Advanced Auth Configuration
// ---------------------------------------------------------------------------

/** A single authentication mode that a skill supports. */
export interface AuthMode {
  type: "managed" | "self_hosted" | "text";
  label?: string;
  description?: string;
  /** Managed mode: OAuth provider name. */
  provider?: string;
  /** Managed mode: OAuth scopes. */
  scopes?: string[];
  /** Managed mode: base URL for API proxying. */
  apiBaseUrl?: string;
  /** Self-hosted mode: form fields for credential input. */
  fields?: SetupField[];
  /** Text mode: hint above the textarea. */
  textDescription?: string;
  /** Text mode: placeholder in the textarea. */
  textPlaceholder?: string;
}

/** Advanced auth configuration with multiple auth modes. */
export interface SkillAuthConfig {
  modes: AuthMode[];
}

// ---------------------------------------------------------------------------
// JSON-RPC 2.0
// ---------------------------------------------------------------------------

export interface JsonRpcRequest {
  jsonrpc: "2.0";
  id: number;
  method: string;
  params?: Record<string, unknown>;
}

export interface JsonRpcResponse {
  jsonrpc: "2.0";
  id: number;
  result?: unknown;
  error?: { code: number; message: string };
}

export interface JsonRpcNotification {
  jsonrpc: "2.0";
  method: string;
  params?: Record<string, unknown>;
}

// ---------------------------------------------------------------------------
// Skill Tools
// ---------------------------------------------------------------------------

export interface SkillToolDefinition {
  name: string;
  description: string;
  inputSchema: {
    type: "object";
    properties: Record<string, unknown>;
    required?: string[];
  };
}

export interface SkillToolResult {
  content: Array<{ type: string; text: string }>;
  isError: boolean;
}

// ---------------------------------------------------------------------------
// Skill Options
// ---------------------------------------------------------------------------

export interface SkillOptionDefinition {
  name: string;
  type: "boolean" | "text" | "number" | "select";
  label: string;
  description?: string | null;
  default?: string | number | boolean | null;
  options?: SetupFieldOption[] | null;
  group?: string | null;
  toolFilter?: string[] | null;
  /** Current value (returned by options/list) */
  value?: string | number | boolean | null;
}

// ---------------------------------------------------------------------------
// Skill Status
// ---------------------------------------------------------------------------

export type SkillStatus =
  | "installed"
  | "starting"
  | "running"
  | "setup_required"
  | "setup_in_progress"
  | "ready"
  | "error"
  | "stopping";

// ---------------------------------------------------------------------------
// Skill Connection Status (unified status derived from skill-pushed state)
// ---------------------------------------------------------------------------

/**
 * Unified connection status for display in the UI.
 * Derived from the skill's own `connection_status` + `auth_status`
 * (pushed via reverse RPC state/set) combined with lifecycle status.
 */
export type SkillConnectionStatus =
  | "connected"          // Service fully connected and authenticated
  | "connecting"         // In the process of connecting or authenticating
  | "not_authenticated"  // Process running, connected to service, but not authed
  | "disconnected"       // Process running but not connected to service
  | "error"              // Connection or auth error
  | "offline"            // Skill process not running
  | "setup_required";    // Needs initial setup

/**
 * Standard state fields that skills should push via state/set reverse RPC.
 * Each skill maps its internal status to these fields.
 */
export interface SkillHostConnectionState {
  connection_status?: "disconnected" | "connecting" | "connected" | "error";
  auth_status?: "not_authenticated" | "authenticating" | "authenticated" | "error";
  connection_error?: string | null;
  auth_error?: string | null;
  is_initialized?: boolean;
}

// ---------------------------------------------------------------------------
// Redux skill state shape
// ---------------------------------------------------------------------------

export interface OAuthCredential {
  credentialId: string;
  provider: string;
  grantedScopes?: string[];
}

// ---------------------------------------------------------------------------
// Ping / Health-Check
// ---------------------------------------------------------------------------

export interface PingResult {
  ok: boolean;
  errorType?: "network" | "auth";
  errorMessage?: string;
}

export interface SkillState {
  manifest: SkillManifest;
  status: SkillStatus;
  error?: string;
  setupComplete: boolean;
  tools: SkillToolDefinition[];
  /** Persisted OAuth credential so it survives app restarts. */
  oauthCredential?: OAuthCredential;
}
