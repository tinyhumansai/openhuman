/**
 * flowsApi.importFlow (Phase 4d) — the host-validated import client. Asserts it
 * forwards the graph + format to `openhuman.flows_import`, unwraps the
 * CLI-compatible `{ result, logs }` envelope, and surfaces the normalized graph
 * plus warnings. Also covers the auto-detect default and error propagation.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { importFlow } from '../flowsApi';

vi.mock('../../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('flowsApi.importFlow', () => {
  beforeEach(async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockReset();
  });

  it('forwards the graph with the default auto format and unwraps the envelope', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    const graph = { schema_version: 1, name: 'Imported', nodes: [], edges: [] };
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      result: { graph, warnings: ['Node X unmapped'] },
      logs: ['flow imported'],
    });

    const result = await importFlow({ some: 'n8n json' });

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.flows_import',
      params: { graph: { some: 'n8n json' }, format: 'auto' },
    });
    expect(result.graph).toEqual(graph);
    expect(result.warnings).toEqual(['Node X unmapped']);
  });

  it('passes an explicit n8n format through', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      result: { graph: { nodes: [] }, warnings: [] },
      logs: ['flow imported'],
    });

    await importFlow({ nodes: [] }, 'n8n');

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.flows_import',
      params: { graph: { nodes: [] }, format: 'n8n' },
    });
  });

  it('propagates a rejection from an invalid definition', async () => {
    const { callCoreRpc } = await import('../../coreRpcClient');
    vi.mocked(callCoreRpc).mockRejectedValueOnce(new Error('missing trigger'));

    await expect(importFlow({ bad: true })).rejects.toThrow('missing trigger');
  });
});
