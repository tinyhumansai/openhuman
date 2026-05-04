import { beforeEach, describe, expect, it, vi } from 'vitest';

import { disableTrigger, enableTrigger, listAvailableTriggers, listTriggers } from './composioApi';

const mockCallCoreRpc = vi.fn();

vi.mock('../../services/coreRpcClient', () => ({
  callCoreRpc: (args: unknown) => mockCallCoreRpc(args),
}));

describe('composioApi trigger wrappers', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('listAvailableTriggers passes toolkit + optional connection_id and unwraps the envelope', async () => {
    mockCallCoreRpc.mockResolvedValue({
      result: { triggers: [{ slug: 'GMAIL_NEW_GMAIL_MESSAGE', scope: 'static' }] },
      logs: ['composio: 1 available trigger(s) for toolkit gmail'],
    });

    const out = await listAvailableTriggers('gmail', 'conn_1');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.composio_list_available_triggers',
      params: { toolkit: 'gmail', connection_id: 'conn_1' },
    });
    expect(out.triggers).toHaveLength(1);
    expect(out.triggers[0].scope).toBe('static');
  });

  it('listAvailableTriggers omits connection_id when not provided', async () => {
    mockCallCoreRpc.mockResolvedValue({ triggers: [] });
    await listAvailableTriggers('gmail');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.composio_list_available_triggers',
      params: { toolkit: 'gmail' },
    });
  });

  it('listTriggers omits filters when no toolkit is given', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: { triggers: [] }, logs: [] });
    await listTriggers();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.composio_list_triggers',
      params: {},
    });
  });

  it('listTriggers forwards toolkit filter', async () => {
    mockCallCoreRpc.mockResolvedValue({ triggers: [] });
    await listTriggers('gmail');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.composio_list_triggers',
      params: { toolkit: 'gmail' },
    });
  });

  it('enableTrigger forwards trigger_config when provided', async () => {
    mockCallCoreRpc.mockResolvedValue({
      result: { triggerId: 'ti_1', slug: 'GMAIL_NEW_GMAIL_MESSAGE', connectionId: 'c1' },
      logs: [],
    });

    const out = await enableTrigger('c1', 'GMAIL_NEW_GMAIL_MESSAGE', { labelIds: 'INBOX' });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.composio_enable_trigger',
      params: {
        connection_id: 'c1',
        slug: 'GMAIL_NEW_GMAIL_MESSAGE',
        trigger_config: { labelIds: 'INBOX' },
      },
    });
    expect(out.triggerId).toBe('ti_1');
  });

  it('enableTrigger omits trigger_config when not provided', async () => {
    mockCallCoreRpc.mockResolvedValue({ triggerId: 'ti_2', slug: 'X', connectionId: 'c1' });
    await enableTrigger('c1', 'X');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.composio_enable_trigger',
      params: { connection_id: 'c1', slug: 'X' },
    });
  });

  it('disableTrigger forwards trigger_id', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: { deleted: true }, logs: [] });
    const out = await disableTrigger('ti_1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.composio_disable_trigger',
      params: { trigger_id: 'ti_1' },
    });
    expect(out.deleted).toBe(true);
  });
});
