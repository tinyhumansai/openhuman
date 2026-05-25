import { beforeEach, describe, expect, it, vi } from 'vitest';

import { channelConnectionsApi } from './channelConnectionsApi';

const mockCallCoreRpc = vi.fn();

vi.mock('../coreRpcClient', () => ({ callCoreRpc: (args: unknown) => mockCallCoreRpc(args) }));

describe('channelConnectionsApi.disconnectChannel', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('calls channels_disconnect with channel and authMode', async () => {
    mockCallCoreRpc.mockResolvedValue({});
    await channelConnectionsApi.disconnectChannel('telegram', 'bot_token');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.channels_disconnect',
      params: { channel: 'telegram', authMode: 'bot_token' },
    });
  });

  it('forwards clearMemory=true to the RPC', async () => {
    mockCallCoreRpc.mockResolvedValue({});
    await channelConnectionsApi.disconnectChannel('discord', 'bot_token', { clearMemory: true });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.channels_disconnect',
      params: { channel: 'discord', authMode: 'bot_token', clearMemory: true },
    });
  });

  it('defaults clearMemory to false when omitted', async () => {
    mockCallCoreRpc.mockResolvedValue({});
    await channelConnectionsApi.disconnectChannel('telegram', 'oauth');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.channels_disconnect',
      params: { channel: 'telegram', authMode: 'oauth' },
    });
  });
});
