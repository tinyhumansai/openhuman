import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('council-registry:api');

export type CouncilSeatMode = 'default' | 'profile' | 'custom';

export interface CouncilSeatDefinition {
  id: number;
  mode: CouncilSeatMode;
  profile_id: string;
  name: string;
  model: string;
  brief: string;
}

export interface CouncilJudgeDefinition {
  mode: CouncilSeatMode;
  profile_id: string;
  name: string;
  model: string;
}

export interface CouncilDefinition {
  id: string;
  name: string;
  description: string;
  jury_count: number;
  debate_rounds: number;
  seats: CouncilSeatDefinition[];
  judge: CouncilJudgeDefinition;
  shared_reasoning: string;
  created_at_ms: number;
  updated_at_ms: number;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
}

function unwrapEnvelope<T>(payload: unknown): T {
  const record = asRecord(payload);
  if (record && 'result' in record && 'logs' in record && Array.isArray(record.logs)) {
    return record.result as T;
  }
  return payload as T;
}

export const councilRegistryApi = {
  list: async (): Promise<CouncilDefinition[]> => {
    log('list councils');
    const payload = await callCoreRpc<unknown>({
      method: 'openhuman.council_registry_list',
      params: {},
    });
    return unwrapEnvelope<CouncilDefinition[]>(payload);
  },

  get: async (id: string): Promise<CouncilDefinition | null> => {
    log('get council id=%s', id);
    const payload = await callCoreRpc<unknown>({
      method: 'openhuman.council_registry_get',
      params: { id },
    });
    return unwrapEnvelope<CouncilDefinition | null>(payload);
  },

  upsert: async (council: CouncilDefinition): Promise<CouncilDefinition> => {
    log('upsert council id=%s name=%s', council.id, council.name);
    const payload = await callCoreRpc<unknown>({
      method: 'openhuman.council_registry_upsert',
      params: { council },
    });
    return unwrapEnvelope<CouncilDefinition>(payload);
  },

  delete: async (id: string): Promise<boolean> => {
    log('delete council id=%s', id);
    const payload = await callCoreRpc<unknown>({
      method: 'openhuman.council_registry_delete',
      params: { id },
    });
    return unwrapEnvelope<boolean>(payload);
  },
};
