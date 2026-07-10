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

describe('channelConnectionsApi default channel (issue #3712)', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('updatePreferences persists the default via channels_set_default', async () => {
    mockCallCoreRpc.mockResolvedValue({ active_channel: 'discord', restart_required: false });
    await channelConnectionsApi.updatePreferences('discord');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.channels_set_default',
      params: { channel: 'discord' },
    });
  });

  it('getDefaultChannel returns the core active_channel', async () => {
    mockCallCoreRpc.mockResolvedValue({ active_channel: 'telegram' });
    const result = await channelConnectionsApi.getDefaultChannel();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.channels_get_default',
      params: {},
    });
    expect(result).toBe('telegram');
  });

  it('getDefaultChannel returns null when active_channel is absent', async () => {
    mockCallCoreRpc.mockResolvedValue({});
    expect(await channelConnectionsApi.getDefaultChannel()).toBeNull();
  });
});
