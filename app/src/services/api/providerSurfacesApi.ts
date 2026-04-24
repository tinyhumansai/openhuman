import type { RespondQueueList } from '../../types/providerSurfaces';
import { callCoreRpc } from '../coreRpcClient';

interface ProviderSurfacesQueueEnvelope {
  data?: RespondQueueList;
  result?: { data?: RespondQueueList };
}

const EMPTY_QUEUE: RespondQueueList = { items: [], count: 0 };

function parseQueueEnvelope(raw: unknown): RespondQueueList {
  if (!raw || typeof raw !== 'object') {
    throw new Error('provider_surfaces_list_queue: unexpected empty response');
  }

  const envelope = raw as ProviderSurfacesQueueEnvelope & { error?: { message?: string } };
  if (envelope.error) {
    throw new Error(envelope.error.message ?? 'Core RPC returned an error');
  }
  const candidate = envelope.result?.data ?? envelope.data;
  if (!candidate || !Array.isArray(candidate.items) || typeof candidate.count !== 'number') {
    return EMPTY_QUEUE;
  }
  return candidate;
}

export const providerSurfacesApi = {
  async listQueue(): Promise<RespondQueueList> {
    const raw = await callCoreRpc<unknown>({ method: 'openhuman.provider_surfaces_list_queue' });
    return parseQueueEnvelope(raw);
  },
};
