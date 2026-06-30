/**
 * Wire shape of the per-thread agent-turn snapshot persisted by the
 * Rust core (`src/openhuman/threads/turn_state/types.rs`). The UI uses
 * these payloads to rehydrate `chatRuntimeSlice` on thread switch and
 * to surface interrupted turns left behind by a previous core process.
 */

export type PersistedTurnLifecycle = 'started' | 'streaming' | 'interrupted' | 'completed';

export type PersistedTurnPhase = 'thinking' | 'tool_use' | 'subagent';

export type PersistedToolStatus = 'running' | 'success' | 'error';

export type TaskBoardCardStatus =
  | 'todo'
  | 'awaiting_approval'
  | 'ready'
  | 'in_progress'
  | 'blocked'
  | 'done'
  | 'rejected';
export type TaskApprovalMode = 'required' | 'not_required';

export interface TaskBoardCard {
  id: string;
  title: string;
  status: TaskBoardCardStatus;
  objective?: string | null;
  plan?: string[];
  assignedAgent?: string | null;
  allowedTools?: string[];
  approvalMode?: TaskApprovalMode | null;
  acceptanceCriteria?: string[];
  evidence?: string[];
  notes?: string | null;
  blocker?: string | null;
  /** Conversation thread id of the card's live/last agent session, if any —
   *  drives the "View session" jump into Conversations. Set by the autonomous
   *  dispatcher and the manual "Work" path. */
  sessionThreadId?: string | null;
  /** Provider/source identifiers for a card ingested from a task source
   *  (`{provider, source_id, external_id, url, repo?, urgency}`); absent on
   *  agent/UI-authored cards. */
  sourceMetadata?: Record<string, unknown> | null;
  order: number;
  updatedAt: string;
}

export interface TaskBoard {
  threadId: string;
  cards: TaskBoardCard[];
  updatedAt: string;
}

export interface PersistedSubagentToolCall {
  callId: string;
  toolName: string;
  status: PersistedToolStatus;
  iteration?: number;
  elapsedMs?: number;
  outputChars?: number;
  /** Server-computed human label for this child call (from `Tool::display_label`). */
  displayName?: string;
  /** Server-computed contextual detail (path / recipient / query). */
  detail?: string;
}

/**
 * One ordered item in the parent turn's processing transcript — the
 * interleaved record of narration, reasoning, and tool calls used to render
 * the "View processing" panel (mirrors the Rust `TranscriptItem`). `seq` is a
 * monotonic per-turn ordering key; a `toolCall` item points at a row in
 * {@link PersistedTurnState.toolTimeline} by `callId`.
 */
export type PersistedTranscriptItem =
  | { kind: 'narration'; round: number; seq: number; text: string }
  | { kind: 'thinking'; round: number; seq: number; text: string }
  | { kind: 'toolCall'; round: number; seq: number; callId: string };

/**
 * One ordered item in a sub-agent's processing transcript (mirrors the Rust
 * `SubagentTranscriptItem`). Unlike the parent transcript there is no `seq` —
 * order is the array order. Persisting these lets the inline sub-agent thoughts
 * survive a settled turn / reload.
 */
export type PersistedSubagentTranscriptItem =
  | { kind: 'thinking'; iteration?: number; text: string }
  | { kind: 'text'; iteration?: number; text: string }
  | {
      kind: 'tool';
      iteration?: number;
      callId: string;
      toolName: string;
      status: PersistedToolStatus;
      elapsedMs?: number;
      outputChars?: number;
      displayName?: string;
      detail?: string;
    };

export interface PersistedSubagentActivity {
  taskId: string;
  agentId: string;
  status?: string;
  mode?: string;
  dedicatedThread?: boolean;
  childIteration?: number;
  childMaxIterations?: number;
  iterations?: number;
  elapsedMs?: number;
  outputChars?: number;
  /** Persistent worker sub-thread id backing this delegation (camelCase from core). */
  workerThreadId?: string;
  toolCalls: PersistedSubagentToolCall[];
  /** Ordered reasoning/narration/tool transcript for this sub-agent — what the
   *  inline thoughts render from. Absent on snapshots written before this
   *  field; the UI then falls back to rebuilding tool-only items. */
  transcript?: PersistedSubagentTranscriptItem[];
}

export interface PersistedToolTimelineEntry {
  id: string;
  name: string;
  round: number;
  status: PersistedToolStatus;
  argsBuffer?: string;
  displayName?: string;
  detail?: string;
  sourceToolName?: string;
  subagent?: PersistedSubagentActivity;
}

export interface PersistedTurnState {
  threadId: string;
  requestId: string;
  lifecycle: PersistedTurnLifecycle;
  iteration: number;
  maxIterations: number;
  phase?: PersistedTurnPhase;
  activeTool?: string;
  activeSubagent?: string;
  streamingText: string;
  thinking: string;
  toolTimeline: PersistedToolTimelineEntry[];
  /** Ordered narration/thinking/tool transcript for the processing panel.
   *  Absent on snapshots written before this field. */
  transcript?: PersistedTranscriptItem[];
  taskBoard?: TaskBoard | null;
  startedAt: string;
  updatedAt: string;
}

export interface GetTurnStateResponse {
  turnState?: PersistedTurnState | null;
}

export interface ListTurnStatesResponse {
  turnStates: PersistedTurnState[];
  count: number;
}

export interface ClearTurnStateResponse {
  cleared: boolean;
}

export type AgentRunKind =
  | 'subagent'
  | 'worker_thread'
  | 'background_agent'
  | 'team_member'
  | 'workflow_child';

export type AgentRunStatus =
  | 'pending'
  | 'running'
  | 'awaiting_user'
  | 'paused'
  | 'completed'
  | 'failed'
  | 'cancelled'
  | 'interrupted';

export interface RunTelemetry {
  runId: string;
  inputTokens: number;
  outputTokens: number;
  cachedInputTokens: number;
  costUsd: number;
  elapsedMs?: number | null;
  toolCount: number;
  model?: string | null;
  provider?: string | null;
  error?: string | null;
  updatedAt?: string | null;
}

export interface AgentRun {
  id: string;
  kind: AgentRunKind;
  parentRunId?: string | null;
  parentThreadId?: string | null;
  agentId?: string | null;
  status: AgentRunStatus;
  promptRef?: string | null;
  workerThreadId?: string | null;
  taskBoardId?: string | null;
  taskCardId?: string | null;
  checkpointPath?: string | null;
  checkpoint?: Record<string, unknown> | null;
  summary?: string | null;
  error?: string | null;
  metadata: Record<string, unknown>;
  telemetry?: RunTelemetry | null;
  startedAt: string;
  updatedAt: string;
  completedAt?: string | null;
}

export interface RunEvent {
  runId: string;
  sequence: number;
  eventType: string;
  payload: Record<string, unknown>;
  timestamp: string;
}

export interface AgentRunListResponse {
  runs: AgentRun[];
  count: number;
}

export interface AgentRunGetResponse {
  run?: AgentRun | null;
}

export interface RunEventListResponse {
  events: RunEvent[];
  count: number;
}

export interface GetTaskBoardResponse {
  taskBoard: TaskBoard;
}

export interface PutTaskBoardResponse {
  taskBoard: TaskBoard;
}
