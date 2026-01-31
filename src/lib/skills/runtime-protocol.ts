/**
 * Runtime Protocol — JSON-RPC Protocol Definition for Subprocess Skills
 *
 * Defines the protocol for subprocess-based runtime skills.
 * This is MCP-compatible with skill lifecycle extensions.
 *
 * Transport: JSON-RPC 2.0 over stdin/stdout (same as MCP stdio transport)
 *
 * This file only defines types and constants. The actual subprocess host
 * is Phase 4 (future).
 */

// ---------------------------------------------------------------------------
// JSON-RPC Base Types
// ---------------------------------------------------------------------------

export interface JsonRpcRequest {
  jsonrpc: "2.0";
  id: string | number;
  method: string;
  params?: unknown;
}

export interface JsonRpcNotification {
  jsonrpc: "2.0";
  method: string;
  params?: unknown;
}

export interface JsonRpcResponse {
  jsonrpc: "2.0";
  id: string | number;
  result?: unknown;
  error?: JsonRpcError;
}

export interface JsonRpcError {
  code: number;
  message: string;
  data?: unknown;
}

// ---------------------------------------------------------------------------
// Standard MCP Methods (Tools)
// ---------------------------------------------------------------------------

/** tools/list — List tool definitions provided by this skill */
export const METHOD_TOOLS_LIST = "tools/list" as const;

/** tools/call — Execute a tool */
export const METHOD_TOOLS_CALL = "tools/call" as const;

export interface ToolsListResult {
  tools: Array<{
    name: string;
    description: string;
    inputSchema: {
      type: "object";
      properties: Record<string, unknown>;
      required?: string[];
    };
  }>;
}

export interface ToolsCallParams {
  name: string;
  arguments: Record<string, unknown>;
}

export interface ToolsCallResult {
  content: Array<{
    type: "text";
    text: string;
  }>;
  isError?: boolean;
}

// ---------------------------------------------------------------------------
// Skill Lifecycle Notifications (Custom Extension)
// ---------------------------------------------------------------------------

/** skill/load — Skill should initialize */
export const METHOD_SKILL_LOAD = "skill/load" as const;

/** skill/unload — Skill should clean up */
export const METHOD_SKILL_UNLOAD = "skill/unload" as const;

/** skill/activate — Skill is now active */
export const METHOD_SKILL_ACTIVATE = "skill/activate" as const;

/** skill/deactivate — Skill is being deactivated */
export const METHOD_SKILL_DEACTIVATE = "skill/deactivate" as const;

/** skill/sessionStart — New AI session */
export const METHOD_SKILL_SESSION_START = "skill/sessionStart" as const;

/** skill/sessionEnd — Session ended */
export const METHOD_SKILL_SESSION_END = "skill/sessionEnd" as const;

/** skill/beforeMessage — Pre-process message (can transform) */
export const METHOD_SKILL_BEFORE_MESSAGE = "skill/beforeMessage" as const;

/** skill/afterResponse — Post-process response (can transform) */
export const METHOD_SKILL_AFTER_RESPONSE = "skill/afterResponse" as const;

/** skill/tick — Periodic heartbeat */
export const METHOD_SKILL_TICK = "skill/tick" as const;

export interface SkillLoadParams {
  manifest: {
    id: string;
    name: string;
    version: string;
  };
  dataDir: string;
}

export interface SkillSessionParams {
  sessionId: string;
}

export interface SkillMessageParams {
  message: string;
}

export interface SkillMessageResult {
  /** Transformed message, or null to keep original */
  message: string | null;
}

// ---------------------------------------------------------------------------
// Skill State Methods
// ---------------------------------------------------------------------------

/** state/get — Read skill's state */
export const METHOD_STATE_GET = "state/get" as const;

/** state/set — Write to skill's state */
export const METHOD_STATE_SET = "state/set" as const;

export interface StateGetResult {
  state: Record<string, unknown>;
}

export interface StateSetParams {
  partial: Record<string, unknown>;
}

// ---------------------------------------------------------------------------
// Entity Methods
// ---------------------------------------------------------------------------

/** entities/register — Register entity types */
export const METHOD_ENTITIES_REGISTER = "entities/register" as const;

/** entities/upsert — Create/update an entity */
export const METHOD_ENTITIES_UPSERT = "entities/upsert" as const;

/** entities/search — Search entities */
export const METHOD_ENTITIES_SEARCH = "entities/search" as const;

/** entities/addRelation — Add a relation between entities */
export const METHOD_ENTITIES_ADD_RELATION = "entities/addRelation" as const;

/** entities/addTag — Tag an entity */
export const METHOD_ENTITIES_ADD_TAG = "entities/addTag" as const;

export interface EntitiesRegisterParams {
  entityTypes?: Array<{
    type: string;
    label: string;
    description?: string;
  }>;
  relationTypes?: Array<{
    type: string;
    label: string;
    description?: string;
  }>;
}

export interface EntitiesUpsertParams {
  id?: string;
  type: string;
  source: string;
  sourceId?: string;
  title?: string;
  summary?: string;
  metadata?: Record<string, unknown>;
}

export interface EntitiesSearchParams {
  query: string;
  types?: string[];
  limit?: number;
}

export interface EntitiesAddRelationParams {
  fromEntityId: string;
  toEntityId: string;
  relationType: string;
  metadata?: Record<string, unknown>;
}

export interface EntitiesAddTagParams {
  entityId: string;
  tag: string;
}

// ---------------------------------------------------------------------------
// Intelligence Methods
// ---------------------------------------------------------------------------

/** intelligence/registerRules — Register intelligence rules */
export const METHOD_INTELLIGENCE_REGISTER_RULES =
  "intelligence/registerRules" as const;

/** intelligence/emitEvent — Fire a custom event */
export const METHOD_INTELLIGENCE_EMIT_EVENT =
  "intelligence/emitEvent" as const;

export interface IntelligenceRegisterRulesParams {
  rules: Array<{
    id: string;
    description: string;
    trigger: {
      eventType: string;
      filter?: {
        entityType?: string;
        source?: string;
      };
    };
    action: {
      type: "create_entity" | "create_relation" | "tag_entity" | "custom";
      params?: Record<string, unknown>;
    };
    cooldownMs?: number;
    enabled?: boolean;
  }>;
}

export interface IntelligenceEmitEventParams {
  eventType: string;
  data: unknown;
}

// ---------------------------------------------------------------------------
// All Methods (Union)
// ---------------------------------------------------------------------------

export const ALL_METHODS = [
  METHOD_TOOLS_LIST,
  METHOD_TOOLS_CALL,
  METHOD_SKILL_LOAD,
  METHOD_SKILL_UNLOAD,
  METHOD_SKILL_ACTIVATE,
  METHOD_SKILL_DEACTIVATE,
  METHOD_SKILL_SESSION_START,
  METHOD_SKILL_SESSION_END,
  METHOD_SKILL_BEFORE_MESSAGE,
  METHOD_SKILL_AFTER_RESPONSE,
  METHOD_SKILL_TICK,
  METHOD_STATE_GET,
  METHOD_STATE_SET,
  METHOD_ENTITIES_REGISTER,
  METHOD_ENTITIES_UPSERT,
  METHOD_ENTITIES_SEARCH,
  METHOD_ENTITIES_ADD_RELATION,
  METHOD_ENTITIES_ADD_TAG,
  METHOD_INTELLIGENCE_REGISTER_RULES,
  METHOD_INTELLIGENCE_EMIT_EVENT,
] as const;
