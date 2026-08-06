import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../coreRpcClient';
import { fetchVoiceAgentSignedUrl } from './voiceAgentApi';

vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const mockCall = vi.mocked(callCoreRpc);

describe('voiceAgentApi', () => {
  beforeEach(() => vi.clearAllMocks());

  it('calls the core RPC and maps the snake_case wire shape to camelCase', async () => {
    mockCall.mockResolvedValueOnce({ signed_url: 'wss://x', agent_id: 'a1', user_token: 'tok' });
    const res = await fetchVoiceAgentSignedUrl();
    expect(mockCall).toHaveBeenCalledWith({ method: 'openhuman.voice_agent_signed_url' });
    expect(res).toEqual({ signedUrl: 'wss://x', agentId: 'a1', userToken: 'tok' });
  });

  it('defaults userToken to empty against a backend that predates the binding', async () => {
    mockCall.mockResolvedValueOnce({ signed_url: 'wss://x', agent_id: 'a1' });
    const res = await fetchVoiceAgentSignedUrl();
    expect(res).toEqual({ signedUrl: 'wss://x', agentId: 'a1', userToken: '' });
  });

  it('propagates the core RPC error (e.g. signed out)', async () => {
    mockCall.mockRejectedValueOnce(new Error('no backend session token; sign in first'));
    await expect(fetchVoiceAgentSignedUrl()).rejects.toThrow('no backend session token');
  });
});
