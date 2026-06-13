/**
 * Vitest coverage for the two new cron tauriCommand wrappers added by the
 * skills runner PR: openhumanCronRun and openhumanCronRuns.
 *
 * Follows the same mocking pattern as subconscious.test.ts — isTauri()
 * guard + callCoreRpc mock, no real Tauri runtime.
 */
import { isTauri } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, type Mock, test, vi } from 'vitest';

import { callCoreRpc } from '../../services/coreRpcClient';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('tauriCommands/cron — openhumanCronRun / openhumanCronRuns', () => {
  const mockIsTauri = isTauri as Mock;
  const mockCallCoreRpc = callCoreRpc as Mock;
  let openhumanCronAdd: typeof import('./cron').openhumanCronAdd;
  let openhumanCronRun: typeof import('./cron').openhumanCronRun;
  let openhumanCronRuns: typeof import('./cron').openhumanCronRuns;

  beforeEach(async () => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(true);
    const m = await vi.importActual<typeof import('./cron')>('./cron');
    openhumanCronAdd = m.openhumanCronAdd;
    openhumanCronRun = m.openhumanCronRun;
    openhumanCronRuns = m.openhumanCronRuns;
  });

  afterEach(() => vi.restoreAllMocks());

  describe('openhumanCronAdd', () => {
    const params = { schedule: { kind: 'cron' as const, expr: '*/5 * * * *' }, name: 'test' };

    test('throws when not in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(openhumanCronAdd(params)).rejects.toThrow('Not running in Tauri');
    });

    test('calls cron_add with params', async () => {
      mockCallCoreRpc.mockResolvedValue({ id: 'job-1' });
      await openhumanCronAdd(params);
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({ method: 'openhuman.cron_add' })
      );
    });
  });

  describe('openhumanCronRun', () => {
    test('throws when not in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(openhumanCronRun('job-1')).rejects.toThrow('Not running in Tauri');
    });

    test('calls cron_run with job_id', async () => {
      mockCallCoreRpc.mockResolvedValue({
        job_id: 'job-1',
        status: 'ok',
        duration_ms: 100,
        output: '',
      });
      await openhumanCronRun('job-1');
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({ method: 'openhuman.cron_run', params: { job_id: 'job-1' } })
      );
    });
  });

  describe('openhumanCronRuns', () => {
    test('throws when not in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(openhumanCronRuns('job-1')).rejects.toThrow('Not running in Tauri');
    });

    test('calls cron_runs with job_id and default limit', async () => {
      mockCallCoreRpc.mockResolvedValue({ runs: [] });
      await openhumanCronRuns('job-1');
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.cron_runs',
          params: expect.objectContaining({ job_id: 'job-1', limit: 20 }),
        })
      );
    });

    test('passes custom limit', async () => {
      mockCallCoreRpc.mockResolvedValue({ runs: [] });
      await openhumanCronRuns('job-1', 5);
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({ params: expect.objectContaining({ limit: 5 }) })
      );
    });
  });
});
