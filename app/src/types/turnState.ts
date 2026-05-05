/**
 * Wire shape of the per-thread agent-turn snapshot persisted by the
 * Rust core (`src/openhuman/threads/turn_state/types.rs`). The UI uses
 * these payloads to rehydrate `chatRuntimeSlice` on thread switch and
 * to surface interrupted turns left behind by a previous core process.
 */

export type PersistedTurnLifecycle = 'started' | 'streaming' | 'interrupted';

export type PersistedTurnPhase = 'thinking' | 'tool_use' | 'subagent';

export type PersistedToolStatus = 'running' | 'success' | 'error';

export interface PersistedSubagentToolCall {
  callId: string;
  toolName: string;
  status: PersistedToolStatus;
  iteration?: number;
  elapsedMs?: number;
  outputChars?: number;
}

export interface PersistedSubagentActivity {
  taskId: string;
  agentId: string;
  mode?: string;
  dedicatedThread?: boolean;
  childIteration?: number;
  childMaxIterations?: number;
  iterations?: number;
  elapsedMs?: number;
  outputChars?: number;
  toolCalls: PersistedSubagentToolCall[];
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
