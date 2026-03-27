/**
 * Intelligence Chat API Types
 *
 * TypeScript definitions for backend integration
 * These types ensure consistency between frontend and backend implementations
 */

// ===== Core Types =====

export type ConversationFlow =
  | 'discovery'
  | 'planning'
  | 'confirmation'
  | 'execution'
  | 'completion'
  | 'auto_close';

export type MessageType =
  | 'message'
  | 'plan'
  | 'progress'
  | 'completion'
  | 'discovery_question'
  | 'confirmation_request';

export type TaskStatus = 'pending' | 'in_progress' | 'completed' | 'failed';

export type SessionStatus = 'active' | 'executing' | 'completed' | 'failed';

export type WorkflowType =
  | 'meeting_prep'
  | 'email_response'
  | 'system_task'
  | 'document_analysis'
  | 'calendar_management';

// ===== Request/Response Types =====

export interface CreateChatSessionRequest {
  actionable_item_id: string;
  user_id: string;
  context: { item_type: WorkflowType; metadata: Record<string, unknown> };
}

export interface CreateChatSessionResponse {
  success: true;
  data: {
    session_id: string;
    initial_message: {
      content: string;
      type: MessageType;
      options: string[];
      context: Record<string, unknown>;
    };
    expected_flow: ConversationFlow[];
  };
}

export interface ChatMessage {
  id: string;
  content: string;
  sender: 'ai' | 'user';
  timestamp: string; // ISO 8601
  type: MessageType;
  metadata?: Record<string, unknown>;
}

export interface ChatSessionDetails {
  session_id: string;
  status: SessionStatus;
  current_flow: ConversationFlow;
  messages: ChatMessage[];
  context: Record<string, unknown>;
  execution_id?: string;
}

export interface SendMessageRequest {
  content: string;
  sender: 'user';
  current_flow: ConversationFlow;
  context: Record<string, unknown>;
}

export interface SendMessageResponse {
  success: true;
  data: {
    ai_response: {
      content: string;
      type: MessageType;
      next_flow: ConversationFlow;
      options: string[];
      metadata: Record<string, unknown>;
    };
    should_execute: boolean;
    execution_plan?: ExecutionPlan;
  };
}

// ===== Execution Types =====

export interface ExecutionStep {
  id: string;
  label: string;
  status: TaskStatus;
  estimated_duration: number;
  dependencies: string[];
  started_at?: string; // ISO 8601
  completed_at?: string; // ISO 8601
  progress_percentage?: number;
  result?: Record<string, unknown>;
}

export interface ExecutionPlan {
  id: string;
  steps: ExecutionStep[];
  estimated_total_duration: number;
  requirements: { gmail_access?: boolean; notion_access?: boolean; calendar_access?: boolean };
}

export interface StartExecutionRequest {
  execution_plan_id: string;
  confirmed: true;
  modifications?: Record<string, unknown>;
}

export interface StartExecutionResponse {
  success: true;
  data: {
    execution_id: string;
    status: 'started';
    estimated_duration: number;
    steps: ExecutionStep[];
  };
}

export interface ExecutionStatusResponse {
  success: true;
  data: {
    execution_id: string;
    status: 'running' | 'completed' | 'failed';
    current_step?: { id: string; label: string; status: TaskStatus; progress_percentage: number };
    completed_steps: string[];
    results?: ExecutionResults;
  };
}

export interface ExecutionResults {
  summary: string;
  artifacts: Artifact[];
  metrics: {
    total_duration: number;
    emails_processed?: number;
    documents_created?: number;
    apis_called?: number;
  };
}

export interface Artifact {
  type: 'notion_doc' | 'email_draft' | 'calendar_event' | 'document' | 'link';
  title: string;
  url: string;
  created_at: string; // ISO 8601
  metadata: Record<string, unknown>;
}

// ===== WebSocket Event Types =====

export interface WebSocketMessage<T = unknown> {
  type: string;
  data: T;
  timestamp?: string; // ISO 8601
}

// Tool definition for chat initialization
export interface ChatTool {
  name: string;
  description: string;
  inputSchema: { type: 'object'; properties: Record<string, unknown>; required?: string[] };
}

// Client → Server Events
export interface ChatInitEvent {
  type: 'chat:init';
  data: { tools: ChatTool[]; sessionId?: string; timestamp: number };
}

export interface AuthenticateEvent {
  type: 'authenticate';
  data: { token: string; session_id: string };
}

export interface JoinSessionEvent {
  type: 'join_session';
  data: { session_id: string };
}

// Server → Client Events
export interface AuthenticatedEvent {
  type: 'authenticated';
  data: { user_id: string; session_id: string };
}

export interface ExecutionStepProgressEvent {
  type: 'execution_step_progress';
  data: {
    session_id: string;
    execution_id: string;
    step_id: string;
    status: TaskStatus;
    progress_percentage: number;
    message: string;
    timestamp: string; // ISO 8601
  };
}

export interface ExecutionCompleteEvent {
  type: 'execution_complete';
  data: {
    session_id: string;
    execution_id: string;
    status: 'completed' | 'failed';
    results: ExecutionResults;
  };
}

export interface AIThinkingEvent {
  type: 'ai_thinking';
  data: { session_id: string; message: string; estimated_time: number };
}

export interface ErrorEvent {
  type: 'error';
  data: {
    session_id: string;
    error_code: ErrorCode;
    message: string;
    details: Record<string, unknown>;
    retry_after?: number;
  };
}

// ===== Error Types =====

export type ErrorCode =
  | 'SESSION_NOT_FOUND'
  | 'EXECUTION_FAILED'
  | 'RATE_LIMITED'
  | 'INVALID_INPUT'
  | 'SOURCE_UNAVAILABLE'
  | 'AI_UNAVAILABLE'
  | 'TIMEOUT'
  | 'UNAUTHORIZED'
  | 'FORBIDDEN'
  | 'INTERNAL_ERROR';

export interface APIError {
  code: ErrorCode;
  message: string;
  details: Record<string, unknown>;
}

// ===== Standard API Response Envelope =====

export interface APIResponse<T = unknown> {
  success: boolean;
  data: T | null;
  error: APIError | null;
  meta: {
    timestamp: string; // ISO 8601
    request_id: string;
    rate_limit: {
      remaining: number;
      reset_at: string; // ISO 8601
    };
  };
}

// ===== Meeting Preparation Specific Types =====

export interface MeetingPrepWorkflow {
  workflow_type: 'meeting_preparation';
  inputs: {
    meeting_title: string;
    participant: string;
    time_context: string;
    document_sources: ('gmail' | 'notion' | 'calendar')[];
    output_format: 'notion_doc' | 'email_summary' | 'both';
  };
  processing_steps: MeetingPrepStep[];
}

export interface MeetingPrepStep {
  step: 'fetch_gmail' | 'access_notion' | 'analyze_context' | 'consolidate_docs' | 'generate_link';
  action: string;
  params: Record<string, unknown>;
}

// ===== Service Integration Types =====

export interface ExternalServiceConfig {
  gmail?: { enabled: boolean; scopes: string[]; rate_limit: number };
  notion?: { enabled: boolean; workspace_id: string; rate_limit: number };
  calendar?: { enabled: boolean; calendar_ids: string[] };
}

export interface ServiceAuthStatus {
  gmail: 'connected' | 'disconnected' | 'error';
  notion: 'connected' | 'disconnected' | 'error';
  calendar: 'connected' | 'disconnected' | 'error';
}

// ===== Performance & Monitoring Types =====

export interface PerformanceMetrics {
  response_time: number;
  execution_time: number;
  external_api_calls: number;
  tokens_used: number;
  cost_estimate: number;
}

export interface SessionMetrics {
  session_id: string;
  user_id: string;
  started_at: string; // ISO 8601
  ended_at?: string; // ISO 8601
  total_messages: number;
  execution_count: number;
  performance: PerformanceMetrics;
  satisfaction_score?: number;
}

// ===== Type Guards =====

export function isWebSocketMessage<T>(obj: unknown): obj is WebSocketMessage<T> {
  if (typeof obj !== 'object' || obj === null) return false;
  const o = obj as Record<string, unknown>;
  return typeof o.type === 'string' && 'data' in o;
}

export function isAPIResponse<T>(obj: unknown): obj is APIResponse<T> {
  return (
    typeof obj === 'object' &&
    obj !== null &&
    typeof (obj as { success?: unknown }).success === 'boolean' &&
    ('data' in (obj as object) || 'error' in (obj as object)) &&
    'meta' in (obj as object)
  );
}

export function isExecutionStepProgressEvent(obj: unknown): obj is ExecutionStepProgressEvent {
  if (!isWebSocketMessage(obj) || obj.type !== 'execution_step_progress') return false;
  const d = obj.data;
  return (
    typeof d === 'object' &&
    d !== null &&
    'step_id' in d &&
    'progress_percentage' in d
  );
}

// ===== Utility Types =====

export type ChatEventHandler<T = unknown> = (event: WebSocketMessage<T>) => void;

export interface ChatClientConfig {
  websocket_url: string;
  api_base_url: string;
  auth_token: string;
  retry_attempts: number;
  timeout_ms: number;
}

// ===== Frontend State Types =====

export interface ChatState {
  session: ChatSessionDetails | null;
  messages: ChatMessage[];
  currentFlow: ConversationFlow;
  isExecuting: boolean;
  executionProgress: ExecutionStep[];
  isConnected: boolean;
  lastError: APIError | null;
}

export interface ChatActions {
  sendMessage: (content: string) => Promise<void>;
  startExecution: (planId: string) => Promise<void>;
  connect: () => Promise<void>;
  disconnect: () => void;
  clearError: () => void;
}
