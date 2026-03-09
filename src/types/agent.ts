/**
 * Agent system types for AlphaHuman.
 * Built on top of the existing skill system infrastructure.
 */

import type { SkillToolDefinition } from '../lib/skills/types';
import type { ThreadMessage, Thread } from './thread';

// =============================================================================
// Agent Tool Types (extends skill tools)
// =============================================================================

/**
 * Agent tool schema compatible with OpenAI function calling format
 * and the existing skill tool system
 */
export interface AgentToolSchema {
  type: 'function';
  function: {
    name: string;
    description: string;
    parameters: {
      type: 'object';
      properties: Record<string, AgentToolParameter>;
      required?: string[];
    };
  };
}

export interface AgentToolParameter {
  type: 'string' | 'number' | 'boolean' | 'array' | 'object';
  description?: string;
  enum?: string[];
  items?: AgentToolParameter;
  properties?: Record<string, AgentToolParameter>;
  required?: string[];
  default?: any;
  minimum?: number;
  maximum?: number;
  pattern?: string;
}

/**
 * Tool execution tracking for agent conversations
 */
export interface AgentToolExecution {
  id: string;
  toolName: string;
  skillId: string; // Which skill provides this tool
  arguments: string; // JSON string
  result?: string;
  status: AgentToolExecutionStatus;
  startTime: number;
  endTime?: number;
  executionTimeMs?: number;
  errorMessage?: string;
  metadata?: {
    retryCount?: number;
    approvalRequired?: boolean;
    approvalGranted?: boolean;
  };
}

export type AgentToolExecutionStatus =
  | 'pending'
  | 'running'
  | 'success'
  | 'error'
  | 'cancelled'
  | 'timeout';

// =============================================================================
// Agent Execution Types
// =============================================================================

/**
 * Configuration options for agent task execution
 */
export interface AgentExecutionOptions {
  maxIterations?: number;
  timeoutMs?: number;
  requireApproval?: boolean;
  allowedSkills?: string[]; // Skill IDs that are allowed to execute
  blockedTools?: string[]; // Specific tools that are blocked
  retryFailedTools?: boolean;
}

/**
 * Result of an agent task execution
 */
export interface AgentExecutionResult {
  status: AgentExecutionStatus;
  executionId: string;
  finalResponse?: string;
  iterations: number;
  toolExecutions: AgentToolExecution[];
  executionTime: number;
  error?: string;
  metadata?: {
    tokensUsed?: number;
    apiCalls?: number;
    toolsAvailable?: number;
    skillsInvolved?: string[];
  };
}

export type AgentExecutionStatus =
  | 'completed'
  | 'timeout'
  | 'error'
  | 'max_iterations'
  | 'cancelled'
  | 'blocked';

/**
 * Active agent execution tracking
 */
export interface AgentExecution {
  id: string;
  threadId: string;
  userMessage: string;
  status: 'initializing' | 'running' | 'completing';
  currentIteration: number;
  maxIterations: number;
  toolExecutions: AgentToolExecution[];
  startTime: number;
  lastUpdate: number;
  abortController?: AbortController;
}

// =============================================================================
// OpenAI API Compatibility Types
// =============================================================================

/**
 * OpenAI-compatible message format for backend communication
 */
export interface OpenAIMessage {
  role: 'system' | 'user' | 'assistant' | 'tool';
  content: string | null;
  name?: string;
  tool_calls?: OpenAIToolCall[];
  tool_call_id?: string;
}

export interface OpenAIToolCall {
  id: string;
  type: 'function';
  function: {
    name: string;
    arguments: string; // JSON string
  };
}

export interface OpenAITool {
  type: 'function';
  function: {
    name: string;
    description: string;
    parameters: any; // JSON Schema
  };
}

/**
 * Chat completion request sent to backend
 */
export interface AgentChatRequest {
  model: string;
  messages: OpenAIMessage[];
  tools?: OpenAITool[];
  tool_choice?: 'auto' | 'none' | 'required';
  temperature?: number;
  max_tokens?: number;
}

/**
 * Chat completion response from backend
 */
export interface AgentChatResponse {
  id: string;
  object: 'chat.completion';
  created: number;
  model: string;
  choices: AgentChatChoice[];
  usage?: {
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
  };
}

export interface AgentChatChoice {
  index: number;
  message: OpenAIMessage;
  finish_reason: 'stop' | 'length' | 'tool_calls' | 'content_filter';
}

// =============================================================================
// Thread System Integration
// =============================================================================

/**
 * Enhanced thread message with agent execution metadata
 */
export interface AgentThreadMessage extends ThreadMessage {
  // Existing ThreadMessage fields remain the same
  // Enhanced extraMetadata for agent tracking
  extraMetadata: ThreadMessage['extraMetadata'] & {
    agentExecutionId?: string;
    toolExecutions?: AgentToolExecution[];
    iterationNumber?: number;
    agentStatus?: AgentExecutionStatus;
  };
}

/**
 * Enhanced thread with agent mode capability
 */
export interface AgentThread extends Thread {
  // Existing Thread fields remain the same
  // Additional agent-specific metadata
  agentMode?: boolean;
  lastAgentExecution?: string;
  agentConfig?: AgentExecutionOptions;
}

// =============================================================================
// Redux State Types
// =============================================================================

/**
 * Agent Redux state that integrates with existing skill system
 */
export interface AgentState {
  // Agent mode enabled per thread
  agentModeByThreadId: Record<string, boolean>;

  // Active agent executions
  activeExecutions: Record<string, AgentExecution>;

  // Agent execution history (persisted)
  executionHistory: AgentExecutionHistoryEntry[];

  // Agent configuration per thread (persisted)
  configByThreadId: Record<string, AgentExecutionOptions>;

  // Tool registry cache (derived from skills)
  toolRegistry: {
    tools: AgentToolSchema[];
    lastUpdated: number;
    loading: boolean;
    error?: string;
  };

  // UI state (not persisted)
  ui: {
    showExecutionDetails: Record<string, boolean>;
    selectedExecution?: string;
  };
}

export interface AgentExecutionHistoryEntry {
  executionId: string;
  threadId: string;
  userMessage: string;
  result: AgentExecutionResult;
  timestamp: number;
  duration: number;
}

// =============================================================================
// Service Interface Types
// =============================================================================

/**
 * Tool registry service interface
 */
export interface IAgentToolRegistry {
  loadToolSchemas(forceReload?: boolean): Promise<AgentToolSchema[]>;
  executeTool(skillId: string, toolName: string, toolArguments: string): Promise<AgentToolExecution>;
  getToolByName(toolName: string): AgentToolSchema | undefined;
  getAllTools(): AgentToolSchema[];
  getToolsBySkill(): Record<string, AgentToolSchema[]>;
}

/**
 * Agent loop service interface
 */
export interface IAgentLoop {
  executeTask(
    userMessage: string,
    threadId: string,
    options?: AgentExecutionOptions
  ): Promise<AgentExecutionResult>;

  cancelExecution(executionId: string): boolean;
  getActiveExecutions(): string[];
  getExecutionStatus(executionId: string): AgentExecution | null;
}

// =============================================================================
// Event Types
// =============================================================================

/**
 * Agent execution events for real-time UI updates
 */
export type AgentEvent =
  | AgentExecutionStartedEvent
  | AgentIterationStartedEvent
  | AgentToolExecutionStartedEvent
  | AgentToolExecutionCompletedEvent
  | AgentExecutionCompletedEvent
  | AgentExecutionErrorEvent;

export interface AgentExecutionStartedEvent {
  type: 'AGENT_EXECUTION_STARTED';
  executionId: string;
  threadId: string;
  userMessage: string;
  timestamp: number;
}

export interface AgentIterationStartedEvent {
  type: 'AGENT_ITERATION_STARTED';
  executionId: string;
  iteration: number;
  timestamp: number;
}

export interface AgentToolExecutionStartedEvent {
  type: 'AGENT_TOOL_EXECUTION_STARTED';
  executionId: string;
  toolExecution: AgentToolExecution;
  timestamp: number;
}

export interface AgentToolExecutionCompletedEvent {
  type: 'AGENT_TOOL_EXECUTION_COMPLETED';
  executionId: string;
  toolExecution: AgentToolExecution;
  timestamp: number;
}

export interface AgentExecutionCompletedEvent {
  type: 'AGENT_EXECUTION_COMPLETED';
  executionId: string;
  result: AgentExecutionResult;
  timestamp: number;
}

export interface AgentExecutionErrorEvent {
  type: 'AGENT_EXECUTION_ERROR';
  executionId: string;
  error: AgentError;
  timestamp: number;
}

// =============================================================================
// Error Types
// =============================================================================

export interface AgentError extends Error {
  type: AgentErrorType;
  code?: string;
  details?: Record<string, any>;
  retryable?: boolean;
}

export type AgentErrorType =
  | 'TOOL_EXECUTION_ERROR'
  | 'TOOL_NOT_FOUND'
  | 'SKILL_NOT_AVAILABLE'
  | 'AGENT_TIMEOUT'
  | 'MAX_ITERATIONS_EXCEEDED'
  | 'API_ERROR'
  | 'VALIDATION_ERROR'
  | 'NETWORK_ERROR'
  | 'UNKNOWN_ERROR';