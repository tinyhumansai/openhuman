/**
 * Frontend client for the user-facing agent registry
 * (`openhuman.agent_registry_*`). Surfaces the shipped default agents plus
 * user-authored custom agents, their enable/disable state, and tool policy.
 *
 * Wire shape note: the Rust handlers return the bare controller payload
 * (RpcOutcome → `into_cli_compatible_json` serializes `data` directly), so
 * `agent_registry_list` resolves to `{ agents }`, the mutating calls to
 * `{ agent }`, and remove to `{ removed }`. Entries serialize with
 * snake_case fields (no serde rename), so the TS shape matches that.
 */
import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('agentRegistryApi');

export type AgentRegistrySource = 'default' | 'custom';

/** Mirror of the Rust `AgentRegistryEntry` (snake_case on the wire). */
export interface AgentRegistryEntry {
  id: string;
  name: string;
  description: string;
  source: AgentRegistrySource;
  enabled: boolean;
  model?: string | null;
  system_prompt?: string | null;
  tool_allowlist?: string[];
  tool_denylist?: string[];
  subagents?: string[];
  tags?: string[];
  metadata?: unknown;
}

/** Fields accepted when creating a custom agent. */
export interface CreateCustomAgentInput {
  id: string;
  name: string;
  description: string;
  enabled?: boolean;
  model?: string | null;
  system_prompt?: string | null;
  tool_allowlist?: string[];
  tool_denylist?: string[];
  subagents?: string[];
  tags?: string[];
}

/** Mirror of the Rust `AgentToolInfo` — one available tool for the picker. */
export interface AgentToolInfo {
  name: string;
  description: string;
}

/** Patch for `update` — any omitted field is left unchanged. */
export interface UpdateAgentInput {
  name?: string;
  description?: string;
  enabled?: boolean;
  model?: string | null;
  system_prompt?: string | null;
  tool_allowlist?: string[];
  tool_denylist?: string[];
  subagents?: string[];
  tags?: string[];
}

function pruneParams(params: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined) out[key] = value;
  }
  return out;
}

export const agentRegistryApi = {
  /** List registry entries (default-first). Includes disabled by default so
   *  the management UI can show and re-enable them. */
  list: async (includeDisabled = true): Promise<AgentRegistryEntry[]> => {
    log('list includeDisabled=%s', includeDisabled);
    const res = await callCoreRpc<{ agents?: AgentRegistryEntry[] }>({
      method: 'openhuman.agent_registry_list',
      params: { include_disabled: includeDisabled },
    });
    return res?.agents ?? [];
  },

  /** List every agent tool visible to the orchestrator, with descriptions.
   *  Used by the agent editor's tool picker; each `name` is a valid
   *  `tool_allowlist` entry. */
  availableTools: async (): Promise<AgentToolInfo[]> => {
    log('availableTools');
    const res = await callCoreRpc<{ tools?: AgentToolInfo[] }>({
      method: 'openhuman.agent_registry_available_tools',
      params: {},
    });
    return res?.tools ?? [];
  },

  /** Fetch a single entry by id. */
  get: async (id: string): Promise<AgentRegistryEntry | null> => {
    log('get id=%s', id);
    const res = await callCoreRpc<{ agent?: AgentRegistryEntry | null }>({
      method: 'openhuman.agent_registry_get',
      params: { id },
    });
    return res?.agent ?? null;
  },

  /** Create (or replace) a custom user-authored agent. */
  createCustom: async (input: CreateCustomAgentInput): Promise<AgentRegistryEntry> => {
    log('createCustom id=%s', input.id);
    const res = await callCoreRpc<{ agent: AgentRegistryEntry }>({
      method: 'openhuman.agent_registry_create_custom',
      params: pruneParams({ ...input }),
    });
    return res.agent;
  },

  /** Patch a custom agent or override a default agent. */
  update: async (id: string, patch: UpdateAgentInput): Promise<AgentRegistryEntry> => {
    log('update id=%s', id);
    const res = await callCoreRpc<{ agent: AgentRegistryEntry }>({
      method: 'openhuman.agent_registry_update',
      params: pruneParams({ id, ...patch }),
    });
    return res.agent;
  },

  /** Enable or disable an agent (default or custom). */
  setEnabled: async (id: string, enabled: boolean): Promise<AgentRegistryEntry> => {
    log('setEnabled id=%s enabled=%s', id, enabled);
    const res = await callCoreRpc<{ agent: AgentRegistryEntry }>({
      method: 'openhuman.agent_registry_set_enabled',
      params: { id, enabled },
    });
    return res.agent;
  },

  /** Remove a custom agent, or reset a default-agent override. Returns
   *  whether a configured entry was actually removed. */
  remove: async (id: string): Promise<boolean> => {
    log('remove id=%s', id);
    const res = await callCoreRpc<{ removed?: boolean }>({
      method: 'openhuman.agent_registry_remove',
      params: { id },
    });
    return Boolean(res?.removed);
  },
};
