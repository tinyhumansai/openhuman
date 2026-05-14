import type { AgentProfile, AgentProfilesResponse } from '../../types/agentProfile';
import { callCoreRpc } from '../coreRpcClient';

interface Envelope<T> {
  data?: T;
}

function unwrapEnvelope<T>(response: Envelope<T> | T): T {
  if (response && typeof response === 'object' && 'data' in response) {
    const envelope = response as Envelope<T>;
    if (envelope.data === undefined) {
      throw new Error('RPC envelope contains undefined data');
    }
    return envelope.data;
  }
  return response as T;
}

export const agentProfilesApi = {
  list: async (): Promise<AgentProfilesResponse> => {
    const response = await callCoreRpc<Envelope<AgentProfilesResponse>>({
      method: 'openhuman.agent_profiles_list',
    });
    return unwrapEnvelope(response);
  },

  select: async (profileId: string): Promise<AgentProfilesResponse> => {
    const response = await callCoreRpc<Envelope<AgentProfilesResponse>>({
      method: 'openhuman.agent_profile_select',
      params: { profile_id: profileId },
    });
    return unwrapEnvelope(response);
  },

  upsert: async (profile: AgentProfile): Promise<AgentProfilesResponse> => {
    const response = await callCoreRpc<Envelope<AgentProfilesResponse>>({
      method: 'openhuman.agent_profile_upsert',
      params: { profile },
    });
    return unwrapEnvelope(response);
  },

  delete: async (profileId: string): Promise<AgentProfilesResponse> => {
    const response = await callCoreRpc<Envelope<AgentProfilesResponse>>({
      method: 'openhuman.agent_profile_delete',
      params: { profile_id: profileId },
    });
    return unwrapEnvelope(response);
  },
};
