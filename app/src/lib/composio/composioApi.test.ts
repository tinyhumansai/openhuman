import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  deleteConnection,
  disableTrigger,
  enableTrigger,
  listAgentReadyToolkits,
  listAvailableTriggers,
  listConnections,
  listToolkits,
  listTriggers,
  syncConnection,
} from './composioApi';

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

  it('deleteConnection forwards clear_memory only when requested', async () => {
    mockCallCoreRpc.mockResolvedValue({
      result: { deleted: true, memory_chunks_deleted: 3 },
      logs: [],
    });

    const out = await deleteConnection('conn-1', { clearMemory: true });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.composio_delete_connection',
      params: { connection_id: 'conn-1', clear_memory: true },
    });
    expect(out.memory_chunks_deleted).toBe(3);
  });
});

describe('syncConnection', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('dispatches composio_sync with the connection id and default reason=manual', async () => {
    mockCallCoreRpc.mockResolvedValue({
      result: { toolkit: 'gmail', connectionId: 'conn-1', items_ingested: 4 },
      logs: ['stub'],
    });

    const out = await syncConnection('conn-1');

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.composio_sync',
      params: { connection_id: 'conn-1', reason: 'manual' },
    });
    // Outcome envelope is unwrapped to the bare provider payload.
    expect(out).toMatchObject({ toolkit: 'gmail', connectionId: 'conn-1' });
  });

  it('forwards an explicit reason verbatim (periodic / connection_created)', async () => {
    mockCallCoreRpc.mockResolvedValue({});

    await syncConnection('conn-2', 'periodic');
    expect(mockCallCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.composio_sync',
      params: { connection_id: 'conn-2', reason: 'periodic' },
    });

    await syncConnection('conn-3', 'connection_created');
    expect(mockCallCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.composio_sync',
      params: { connection_id: 'conn-3', reason: 'connection_created' },
    });
  });

  it('returns non-object outcomes verbatim (unwrap is a no-op for primitives)', async () => {
    // Defensive: a future Rust handler returning a bare scalar / null
    // shouldn't trip the unwrap path.
    mockCallCoreRpc.mockResolvedValue(null);
    const out = await syncConnection('conn-null');
    expect(out).toBeNull();
  });
});

describe('listAgentReadyToolkits', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('dispatches composio_list_agent_ready_toolkits and unwraps the envelope', async () => {
    mockCallCoreRpc.mockResolvedValue({
      result: { toolkits: ['excel', 'gmail', 'one_drive', 'todoist'] },
      logs: ['composio: 4 agent-ready toolkit(s) listed'],
    });

    const out = await listAgentReadyToolkits();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.composio_list_agent_ready_toolkits',
    });
    expect(out.toolkits).toContain('excel');
    expect(out.toolkits).toContain('one_drive');
    expect(out.toolkits).toContain('todoist');
  });

  it('returns flat payload verbatim when the RPC layer did not wrap it', async () => {
    mockCallCoreRpc.mockResolvedValue({ toolkits: ['gmail'] });
    const out = await listAgentReadyToolkits();
    expect(out.toolkits).toEqual(['gmail']);
  });
});

describe('Connections loading fetches (opt-in bounded timeout)', () => {
  // The Connections page clears its loading skeleton only after BOTH of
  // these settle (Promise.allSettled in useComposioIntegrations), so it opts
  // both into the shorter 8s budget to bound the skeleton window on a cold
  // cache against a down backend (#3933). The catalog is safe to time out
  // early — it has a 24h stale cache plus a hardcoded fallback.
  //
  // The timeout is *opt-in*, not the wrapper default: `listConnections` is
  // shared by the repo/issue pickers, add-memory-source dialog and connect
  // modal poll, where a slow-but-successful 8–30s call must still complete
  // (#4079 review). Default callers therefore pass no `timeoutMs`.
  const EXPECTED_FETCH_TIMEOUT_MS = 8_000;

  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('listToolkits omits timeoutMs by default and unwraps the envelope', async () => {
    mockCallCoreRpc.mockResolvedValue({
      result: { toolkits: ['gmail', 'slack'] },
      logs: ['composio: 2 toolkit(s) listed'],
    });

    const out = await listToolkits();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.composio_list_toolkits' });
    expect(mockCallCoreRpc.mock.calls[0][0]).not.toHaveProperty('timeoutMs');
    expect(out.toolkits).toEqual(['gmail', 'slack']);
  });

  it('listToolkits forwards an explicit timeoutMs option', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: { toolkits: [] }, logs: [] });

    await listToolkits({ timeoutMs: EXPECTED_FETCH_TIMEOUT_MS });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.composio_list_toolkits',
      timeoutMs: EXPECTED_FETCH_TIMEOUT_MS,
    });
  });

  it('listConnections omits timeoutMs by default and unwraps the envelope', async () => {
    mockCallCoreRpc.mockResolvedValue({
      result: { connections: [{ toolkit: 'gmail', status: 'ACTIVE' }] },
      logs: [],
    });

    const out = await listConnections();

    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.composio_list_connections' });
    expect(mockCallCoreRpc.mock.calls[0][0]).not.toHaveProperty('timeoutMs');
    expect(out.connections).toHaveLength(1);
  });

  it('listConnections forwards an explicit timeoutMs option', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: { connections: [] }, logs: [] });

    await listConnections({ timeoutMs: EXPECTED_FETCH_TIMEOUT_MS });

    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.composio_list_connections',
      timeoutMs: EXPECTED_FETCH_TIMEOUT_MS,
    });
  });
});

describe('deleteConnection', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('calls composio_delete_connection with connection_id', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: { deleted: true }, logs: [] });
    await deleteConnection('conn-abc');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.composio_delete_connection',
      params: { connection_id: 'conn-abc' },
    });
  });

  it('forwards clearMemory=true to the RPC', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: { deleted: true }, logs: [] });
    await deleteConnection('conn-abc', { clearMemory: true });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.composio_delete_connection',
      params: { connection_id: 'conn-abc', clear_memory: true },
    });
  });
});
