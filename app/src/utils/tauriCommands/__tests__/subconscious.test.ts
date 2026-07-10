import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../../services/coreRpcClient';
import { isTauri } from '../common';
import { subconsciousStatus, subconsciousTrigger } from '../subconscious';

vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));
vi.mock('../common', () => ({ isTauri: vi.fn(() => true), CommandResponse: undefined }));

describe('subconscious client', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(isTauri).mockReturnValue(true);
  });

  it('trigger with no kind omits params (legacy memory behavior)', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ result: { triggered: true } } as never);
    await subconsciousTrigger();
    expect(callCoreRpc).toHaveBeenCalledWith({ method: 'openhuman.subconscious_trigger' });
  });

  it('trigger passes the kind through as params', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ result: { triggered: true } } as never);
    await subconsciousTrigger('all');
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.subconscious_trigger',
      params: { kind: 'all' },
    });
  });

  it('status parses instances[] and tolerates its absence', async () => {
    // With instances (new core).
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      result: {
        instance: 'memory',
        enabled: true,
        instances: [
          { instance: 'memory', enabled: true },
          { instance: 'tinyplace', enabled: false },
        ],
      },
    } as never);
    const withInstances = await subconsciousStatus();
    expect(withInstances.result?.instances).toHaveLength(2);

    // Without instances (older core) — the top-level fields still parse.
    vi.mocked(callCoreRpc).mockResolvedValueOnce({
      result: { instance: 'memory', enabled: true },
    } as never);
    const legacy = await subconsciousStatus();
    expect(legacy.result?.instances).toBeUndefined();
    expect(legacy.result?.instance).toBe('memory');
  });
});
