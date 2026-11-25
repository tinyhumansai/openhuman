/** TypeScript aliases generated from the persisted Rust session record schemas. */
import type { ThreadWireTypes } from './threadWire.generated';

export type PersistedTurnState = NonNullable<ThreadWireTypes['getTurnState']['turnState']>;
export type PersistedTurnLifecycle = PersistedTurnState['lifecycle'];
export type PersistedTurnPhase = NonNullable<PersistedTurnState['phase']>;
export type PersistedToolTimelineEntry = PersistedTurnState['toolTimeline'][number];
export type PersistedToolStatus = PersistedToolTimelineEntry['status'];
export type PersistedToolFailure = NonNullable<PersistedToolTimelineEntry['failure']>;
export type PersistedTranscriptItem = NonNullable<PersistedTurnState['transcript']>[number];
export type PersistedSubagentActivity = NonNullable<PersistedToolTimelineEntry['subagent']>;
export type PersistedSubagentToolCall = PersistedSubagentActivity['toolCalls'][number];
export type PersistedSubagentTranscriptItem = NonNullable<
  PersistedSubagentActivity['transcript']
>[number];

export type GetTurnStateResponse = ThreadWireTypes['getTurnState'];
export type ListTurnStatesResponse = ThreadWireTypes['listTurnStates'];
export type ClearTurnStateResponse = ThreadWireTypes['clearTurnState'];

export type AgentRun = ThreadWireTypes['agentRun'];
export type AgentRunKind = AgentRun['kind'];
export type AgentRunStatus = AgentRun['status'];
export type RunTelemetry = NonNullable<AgentRun['telemetry']>;
export type RunEvent = ThreadWireTypes['runEvent'];
export type AgentRunListResponse = ThreadWireTypes['agentRunList'];
export type AgentRunGetResponse = ThreadWireTypes['agentRunGet'];
export type RunEventListResponse = ThreadWireTypes['runEventList'];
