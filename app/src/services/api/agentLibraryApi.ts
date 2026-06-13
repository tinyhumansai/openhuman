import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('agentLibraryApi');

export type AgentDefinitionSource = 'builtin' | 'custom';

export interface AgentDefinitionModel {
  kind: 'inherit' | 'exact' | 'hint' | string;
  value?: string | null;
}

export interface AgentDefinitionDisplay {
  id: string;
  display_name: string;
  when_to_use: string;
  tier: 'chat' | 'reasoning' | 'worker' | string;
  model: AgentDefinitionModel;
  direct_tool_count: number;
  direct_tool_names: string[];
  uses_wildcard_tools: boolean;
  subagent_ids: string[];
  includes_profile: boolean;
  includes_memory_md: boolean;
  includes_memory_context: boolean;
  can_run_as_user_facing_worker: boolean;
  write_capable: boolean;
  source: AgentDefinitionSource;
}

export const agentLibraryApi = {
  listDefinitions: async (): Promise<AgentDefinitionDisplay[]> => {
    log('[agent-library] listDefinitions entry');
    const response = await callCoreRpc<{ definitions?: AgentDefinitionDisplay[] }>({
      method: 'openhuman.agent_list_definitions',
      params: {},
    });
    const definitions = Array.isArray(response?.definitions) ? response.definitions : [];
    log('[agent-library] listDefinitions exit count=%d', definitions.length);
    return definitions;
  },
};
