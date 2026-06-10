/**
 * Frontend client for the durable agent-team coordination surface (#3374).
 *
 * Wraps the read paths of the `openhuman.agent_team_*` controller family that
 * landed with the durable team ledger (PR1, #3546): `agent_team_list`,
 * `agent_team_get`, and `agent_team_list_messages`. PR2 is a READ-ONLY surface
 * — creating teams, assigning/claiming tasks and posting messages is the
 * agents' job (and a later PR), so only the three read methods live here.
 *
 * The Rust controllers serialize their row types with
 * `#[serde(rename_all = "camelCase")]`, so the wire payload is already camelCase
 * and no snake/camel transform is needed. What this client DOES own is
 * normalizing the controllers' inconsistent response envelopes — `get` returns
 * `{ team }`, `list` returns `{ teams, count }`, `list_messages` returns
 * `{ messages }` — into the clean shapes the UI consumes. Quarantining that
 * here means components never see the wrapper objects, and if the backend later
 * normalizes its envelopes only this file changes.
 */
import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('agentTeamApi');

/** Lifecycle of a team. Mirrors Rust `AgentTeamStatus`. */
export type AgentTeamStatus = 'active' | 'closed';

/** Lifecycle of a single member. Mirrors Rust `AgentTeamMemberStatus`. */
export type AgentTeamMemberStatus = 'pending' | 'active' | 'idle' | 'stopped';

/** Lifecycle of a coordination task. Mirrors Rust `AgentTeamTaskStatus`. */
export type AgentTeamTaskStatus = 'todo' | 'ready' | 'in_progress' | 'blocked' | 'done';

/** A team header row. Mirrors Rust `AgentTeam`. */
export interface AgentTeam {
  id: string;
  parentThreadId?: string | null;
  leadAgentId: string;
  status: AgentTeamStatus;
  summary?: string | null;
  createdAt: string;
  updatedAt: string;
  closedAt?: string | null;
}

/** A member of a team. Mirrors Rust `AgentTeamMember`. */
export interface AgentTeamMember {
  id: string;
  teamId: string;
  name: string;
  agentId?: string | null;
  memberStatus: AgentTeamMemberStatus;
  currentTaskId?: string | null;
  workerThreadId?: string | null;
  runId?: string | null;
  createdAt: string;
  updatedAt: string;
}

/** A coordination task within a team. Mirrors Rust `AgentTeamTask`. */
export interface AgentTeamTask {
  id: string;
  teamId: string;
  title: string;
  objective?: string | null;
  status: AgentTeamTaskStatus;
  ownerMemberId?: string | null;
  claimedByMemberId?: string | null;
  claimToken?: string | null;
  /** Task ids this task depends on. Unmet (non-`done`) deps block a claim. */
  dependsOn: string[];
  /** Free-form gate status. Known: `pending` / `passed` / `failed`; default `pending`. */
  gateStatus: string;
  gateReason?: string | null;
  evidence: string[];
  sourceRunId?: string | null;
  orderIndex: number;
  createdAt: string;
  updatedAt: string;
}

/** A team plus its members and tasks. Mirrors Rust `TeamView` (the `get` shape). */
export interface TeamView {
  team: AgentTeam;
  members: AgentTeamMember[];
  tasks: AgentTeamTask[];
}

/** Parsed payload of a `team_message` run event. Mirrors the Rust `json!` body. */
export interface TeamMessagePayload {
  from: string;
  to: string | null;
  content: string;
  visibility: string;
}

/** A teammate message. A `team_message` run event with its payload typed. */
export interface TeamMessage {
  runId: string;
  sequence: number;
  eventType: string;
  payload: TeamMessagePayload;
  timestamp: string;
}

/** Raw `RunEvent` wire shape before the payload is narrowed to a message. */
interface RawRunEvent {
  runId: string;
  sequence: number;
  eventType: string;
  payload: unknown;
  timestamp: string;
}

/** Optional filters for {@link agentTeamApi.list}. Mirrors `AgentTeamListRequest`. */
export interface AgentTeamListParams {
  parentThreadId?: string;
  status?: AgentTeamStatus;
  limit?: number;
  offset?: number;
}

function assertPositiveInt(value: number | undefined, label: string): void {
  if (value !== undefined && (!Number.isInteger(value) || value <= 0)) {
    throw new Error(`agentTeamApi: ${label} must be a positive integer`);
  }
}

/** Coerce a raw run-event payload into a typed message payload, defensively. */
function readMessagePayload(payload: unknown): TeamMessagePayload {
  const p = (payload ?? {}) as Record<string, unknown>;
  return {
    from: typeof p.from === 'string' ? p.from : '',
    to: typeof p.to === 'string' ? p.to : null,
    content: typeof p.content === 'string' ? p.content : '',
    visibility: typeof p.visibility === 'string' ? p.visibility : 'team',
  };
}

export const agentTeamApi = {
  /**
   * List team headers, newest first. Filters are optional; `parentThreadId`
   * scopes to one conversation, `status` to active/closed.
   *
   * Unwraps the controller's `{ teams, count }` envelope down to `AgentTeam[]`.
   */
  list: async (params: AgentTeamListParams = {}): Promise<AgentTeam[]> => {
    assertPositiveInt(params.limit, 'limit');
    log('list params=%o', params);
    const response = await callCoreRpc<{ teams?: AgentTeam[]; count?: number }>({
      method: 'openhuman.agent_team_list',
      params,
    });
    const teams = response.teams ?? [];
    log('list received teams=%d count=%o', teams.length, response.count);
    return teams;
  },

  /**
   * Fetch one team plus its members and tasks. Returns `null` when the id is
   * unknown (the controller answers `{ team: null }`).
   */
  get: async (teamId: string): Promise<TeamView | null> => {
    if (!teamId) throw new Error('agentTeamApi.get: teamId is required');
    log('get teamId=%s', teamId);
    const response = await callCoreRpc<{ team: TeamView | null }>({
      method: 'openhuman.agent_team_get',
      params: { teamId },
    });
    log('get found=%o', response.team != null);
    return response.team ?? null;
  },

  /**
   * List a team's teammate messages in sequence order. Unwraps the
   * `{ messages }` envelope and narrows each run-event payload to a typed
   * {@link TeamMessagePayload}.
   */
  listMessages: async (teamId: string, limit?: number): Promise<TeamMessage[]> => {
    if (!teamId) throw new Error('agentTeamApi.listMessages: teamId is required');
    assertPositiveInt(limit, 'limit');
    log('listMessages teamId=%s limit=%o', teamId, limit);
    const response = await callCoreRpc<{ messages?: RawRunEvent[] }>({
      method: 'openhuman.agent_team_list_messages',
      params: limit === undefined ? { teamId } : { teamId, limit },
    });
    const messages = (response.messages ?? []).map(event => ({
      runId: event.runId,
      sequence: event.sequence,
      eventType: event.eventType,
      payload: readMessagePayload(event.payload),
      timestamp: event.timestamp,
    }));
    log('listMessages received=%d', messages.length);
    return messages;
  },
};
