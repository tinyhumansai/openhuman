import { callCoreRpc } from './coreRpcClient';

export interface CoreCommandResponse<T> {
  result: T;
  logs: string[];
}

export async function callCoreCommand<T>(method: string, params?: unknown): Promise<T> {
  const response = await callCoreRpc<CoreCommandResponse<T>>({ method, params });
  return response.result;
}
