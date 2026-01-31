/**
 * Unified Skill System — Core Type Definitions
 *
 * All interfaces for the skill system including lifecycle management,
 * tool definitions, state management, entity extensions, and intelligence rules.
 */

import type { MCPTool, MCPToolResult } from "../mcp/types";
import type { ToolTier } from "../mcp/rateLimiter";
import type { EntityType, EntitySource, RelationType } from "../ai/entities/types";
import type { MemoryManager } from "../ai/memory/manager";
import type { SessionManager } from "../ai/sessions/manager";
import type { EntityManager } from "../ai/entities/manager";
import type { ApiClient } from "../../services/apiClient";

// Re-export ToolTier from rateLimiter so skills can reference it
export type { ToolTier };

// ---------------------------------------------------------------------------
// Skill Lifecycle
// ---------------------------------------------------------------------------

/** Skill status lifecycle states */
export type SkillStatus =
  | "registered"
  | "loading"
  | "initialized"
  | "active"
  | "deactivating"
  | "error"
  | "disabled";

/** Skill execution tier */
export type SkillTier = "bundled" | "runtime";

// ---------------------------------------------------------------------------
// Skill Manifest
// ---------------------------------------------------------------------------

/** Skill manifest (metadata) */
export interface SkillManifest {
  /** Unique skill identifier */
  id: string;
  /** Human-readable name */
  name: string;
  /** Description of what the skill does */
  description: string;
  /** Semantic version */
  version: string;
  /** Whether this skill is bundled (build-time) or runtime (subprocess) */
  tier: SkillTier;
  /** Other skill IDs this depends on */
  dependencies?: string[];
  /** Required platform features */
  capabilities?: string[];
  /** Icon identifier or path */
  icon?: string;
  /** Skill author */
  author?: string;
  /** Runtime skills only: subprocess configuration */
  runtime?: {
    /** Command to execute (e.g., "python3", "deno run") */
    command: string;
    /** Arguments to pass (e.g., ["skill.py"]) */
    args?: string[];
    /** Environment variables */
    env?: Record<string, string>;
  };
}

// ---------------------------------------------------------------------------
// Lifecycle Hooks
// ---------------------------------------------------------------------------

/** Enhanced lifecycle hooks that a skill can implement */
export interface SkillHooks {
  /** Called when skill is loaded */
  onLoad?(ctx: SkillRuntimeContext): Promise<void>;
  /** Called when skill is unloaded */
  onUnload?(ctx: SkillRuntimeContext): Promise<void>;
  /** Called when skill becomes active */
  onActivate?(ctx: SkillRuntimeContext): Promise<void>;
  /** Called when skill is being deactivated */
  onDeactivate?(ctx: SkillRuntimeContext): Promise<void>;
  /** Called when a new AI session starts */
  onSessionStart?(ctx: SkillRuntimeContext, sessionId: string): Promise<void>;
  /** Called when an AI session ends */
  onSessionEnd?(ctx: SkillRuntimeContext, sessionId: string): Promise<void>;
  /** Called before the AI processes a user message; can transform it */
  onBeforeMessage?(ctx: SkillRuntimeContext, message: string): Promise<string | void>;
  /** Called after the AI generates a response; can transform it */
  onAfterResponse?(ctx: SkillRuntimeContext, response: string): Promise<string | void>;
  /** Called before memory compaction */
  onMemoryFlush?(ctx: SkillRuntimeContext): Promise<void>;
  /** Called on a schedule (tickInterval ms) while active */
  onTick?(ctx: SkillRuntimeContext): Promise<void>;
  /** Called when connection status changes */
  onConnectionChange?(ctx: SkillRuntimeContext, status: string): Promise<void>;
}

// ---------------------------------------------------------------------------
// Tool Definitions
// ---------------------------------------------------------------------------

/** Unified tool definition used across all systems */
export interface SkillToolDefinition {
  /** Standard MCP tool definition */
  tool: MCPTool;
  /** Execution function */
  handler: SkillToolHandler;
  /** Rate limit classification */
  tier?: ToolTier;
  /** Whether the tool is safe for ask/plan modes */
  readOnly?: boolean;
}

/** Tool execution handler */
export type SkillToolHandler = (
  args: Record<string, unknown>,
  ctx: SkillToolContext,
) => Promise<MCPToolResult>;

/** Context passed to tool handlers */
export interface SkillToolContext {
  /** The owning skill's manifest */
  skillId: string;
  /** Read the skill's Zustand state */
  getState<S = unknown>(): S;
  /** Write to the skill's Zustand state */
  setState<S = unknown>(partial: Partial<S>): void;
  /** Log a message */
  log(message: string): void;
}

// ---------------------------------------------------------------------------
// State Definition
// ---------------------------------------------------------------------------

/** Skill state definition for Zustand */
export interface SkillStateDefinition<S = unknown> {
  /** Initial state value */
  initialState: S;
  /** Persistence configuration */
  persist?: {
    /** Storage key name */
    name: string;
    /** Fields to persist (default: all) */
    whitelist?: (keyof S & string)[];
    /** Fields to reset on rehydrate */
    volatileKeys?: (keyof S & string)[];
  };
}

// ---------------------------------------------------------------------------
// Entity Extension
// ---------------------------------------------------------------------------

/** Entity type registration from a skill */
export interface EntityTypeRegistration {
  /** The entity type name */
  type: string;
  /** Human-readable label */
  label: string;
  /** Description of this entity type */
  description?: string;
  /** The skill that registered this type */
  skillId: string;
}

/** Relation type registration from a skill */
export interface RelationTypeRegistration {
  /** The relation type name */
  type: string;
  /** Human-readable label */
  label: string;
  /** Description of this relation */
  description?: string;
  /** The skill that registered this type */
  skillId: string;
}

/** Entity builder converts raw data into Entity records */
export interface EntityBuilder {
  /** Source system (e.g., "telegram", "onchain") */
  source: EntitySource | string;
  /** Entity type this builder creates */
  entityType: EntityType | string;
  /** Transform raw data into entity fields */
  build(rawData: unknown): {
    sourceId: string;
    title: string;
    summary?: string;
    metadata?: Record<string, unknown>;
  } | null;
}

/** Entity extension definition for a skill */
export interface SkillEntityDefinition {
  /** New entity types to register */
  entityTypes?: EntityTypeRegistration[];
  /** New relation types to register */
  relationTypes?: RelationTypeRegistration[];
  /** Entity builders for converting raw data */
  builders?: EntityBuilder[];
}

// ---------------------------------------------------------------------------
// Intelligence Rules
// ---------------------------------------------------------------------------

/** Trigger conditions for intelligence rules */
export interface IntelligenceTrigger {
  /** Event type to match (e.g., "entity_created", "message_received") */
  eventType: string;
  /** Optional filter on event data */
  filter?: {
    /** Match on entity type */
    entityType?: string;
    /** Match on source */
    source?: string;
    /** Custom predicate */
    predicate?: (data: unknown) => boolean;
  };
}

/** Actions that intelligence rules can perform */
export interface IntelligenceAction {
  /** Action type */
  type: "create_entity" | "create_relation" | "tag_entity" | "custom";
  /** Parameters for the action */
  params?: Record<string, unknown>;
  /** Custom handler for "custom" action type */
  handler?: (data: unknown, ctx: IntelligenceActionContext) => Promise<void>;
}

/** Context passed to intelligence action handlers */
export interface IntelligenceActionContext {
  /** Entity manager for creating/updating entities */
  entities: EntityManager;
  /** Log a message */
  log(message: string): void;
}

/** An intelligence rule definition */
export interface IntelligenceRule {
  /** Unique rule ID */
  id: string;
  /** Human-readable description */
  description: string;
  /** What triggers this rule */
  trigger: IntelligenceTrigger;
  /** What happens when triggered */
  action: IntelligenceAction;
  /** Minimum ms between firings (default: no cooldown) */
  cooldownMs?: number;
  /** Whether the rule is enabled (default: true) */
  enabled?: boolean;
}

/** Intelligence definition for a skill */
export interface SkillIntelligenceDefinition {
  rules: IntelligenceRule[];
}

// ---------------------------------------------------------------------------
// Skill Definition (Complete)
// ---------------------------------------------------------------------------

/** The complete skill definition — everything a skill provides */
export interface SkillDefinition {
  /** Skill manifest (metadata) */
  manifest: SkillManifest;
  /** SKILL.md content (prompt-only skills) */
  promptContent?: string;
  /** Lifecycle hooks */
  hooks?: SkillHooks;
  /** Tool definitions */
  tools?: SkillToolDefinition[];
  /** Zustand state definition */
  state?: SkillStateDefinition;
  /** Entity type extensions */
  entities?: SkillEntityDefinition;
  /** Intelligence rules */
  intelligence?: SkillIntelligenceDefinition;
  /** Tick interval in ms for onTick hook */
  tickInterval?: number;
  /** Context prompt injected when skill tools are active */
  contextPrompt?: string;
  /** Public methods exposed for inter-skill communication */
  publicMethods?: Record<string, (...args: unknown[]) => Promise<unknown>>;
}

// ---------------------------------------------------------------------------
// Runtime Context
// ---------------------------------------------------------------------------

/** Public API of a skill for inter-skill communication */
export interface SkillPublicAPI {
  manifest: SkillManifest;
  publicMethods: Record<string, (...args: unknown[]) => Promise<unknown>>;
}

/** Reference to the unified tool registry interface */
export interface UnifiedToolRegistry {
  registerTools(skillId: string, tools: SkillToolDefinition[]): void;
  unregisterTools(skillId: string): void;
  listTools(): MCPTool[];
  executeTool(name: string, args: Record<string, unknown>): Promise<MCPToolResult | undefined>;
  getToolOwner(toolName: string): string | undefined;
  getToolTier(toolName: string): ToolTier;
  isReadOnly(toolName: string): boolean;
}

/** Enhanced runtime context passed to skill hooks */
export interface SkillRuntimeContext {
  /** The skill's manifest */
  manifest: SkillManifest;
  /** Memory manager for reading/writing memory files */
  memory: MemoryManager;
  /** Session manager for current session */
  session: SessionManager;
  /** Unified tool registry */
  toolRegistry: UnifiedToolRegistry;
  /** Entity manager for querying the platform graph */
  entities: EntityManager;
  /** Skill's own storage directory path */
  dataDir: string;
  /** Read a file from the skill's data directory */
  readData(filename: string): Promise<string>;
  /** Write a file to the skill's data directory */
  writeData(filename: string, content: string): Promise<void>;
  /** Log a message */
  log(message: string): void;
  /** Backend HTTP client */
  apiClient: ApiClient;
  /** Read the skill's Zustand store */
  getState<S = unknown>(): S;
  /** Write to the skill's Zustand store */
  setState<S = unknown>(partial: Partial<S>): void;
  /** Access another skill's public API */
  getSkill(skillId: string): SkillPublicAPI | undefined;
  /** Emit an event to trigger intelligence rules */
  emitEvent(eventName: string, data: unknown): void;
}

// ---------------------------------------------------------------------------
// Managed Skill (Internal)
// ---------------------------------------------------------------------------

/** Internal state tracked per registered skill */
export interface ManagedSkill {
  definition: SkillDefinition;
  status: SkillStatus;
  error?: string;
  context?: SkillRuntimeContext;
  tickTimer?: ReturnType<typeof setInterval>;
}
