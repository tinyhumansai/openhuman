import { beforeEach, describe, expect, it, vi } from 'vitest';

import { skillsApi } from './skillsApi';

const mockCallCoreRpc = vi.fn();
vi.mock('../coreRpcClient', () => ({ callCoreRpc: (...a: unknown[]) => mockCallCoreRpc(...a) }));

describe('skillsApi', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  describe('createSkill', () => {
    it('includes inputs in params when non-empty', async () => {
      mockCallCoreRpc.mockResolvedValue({
        skill: { id: 's', name: 'S', description: '', scope: 'user' as const },
      });
      await skillsApi.createSkill({
        name: 'S',
        description: 'desc',
        inputs: [{ name: 'repo', type: 'string' as const, description: 'repo', required: true }],
      });
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({ params: expect.objectContaining({ inputs: expect.any(Array) }) })
      );
    });
  });

  describe('describeSkill', () => {
    it('calls openhuman.skills_describe with skill_id', async () => {
      mockCallCoreRpc.mockResolvedValue({
        id: 'dev-workflow',
        name: 'Dev Workflow',
        description: 'Auto dev',
        inputs: [],
      });
      const result = await skillsApi.describeSkill('dev-workflow');
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.skills_describe',
          params: { skill_id: 'dev-workflow' },
        })
      );
      expect(result.id).toBe('dev-workflow');
    });

    it('unwraps data-envelope shape', async () => {
      mockCallCoreRpc.mockResolvedValue({
        data: { id: 'x', name: 'X', description: '', inputs: [], skill_id: 'x' },
      });
      const result = await skillsApi.describeSkill('x');
      expect(result.id).toBe('x');
    });
  });

  describe('runSkill', () => {
    it('calls openhuman.skills_run with skill_id and inputs', async () => {
      mockCallCoreRpc.mockResolvedValue({ run_id: 'run-1', skill_id: 's', log: '/tmp/log' });
      const result = await skillsApi.runSkill('s', { repo: 'owner/repo' });
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.skills_run',
          params: { skill_id: 's', inputs: { repo: 'owner/repo' } },
        })
      );
      expect(result.run_id).toBe('run-1');
    });
  });

  describe('readRunLog', () => {
    it('calls skills_read_run_log with run_id', async () => {
      mockCallCoreRpc.mockResolvedValue({
        bytes_read: 100,
        eof: false,
        complete: false,
        content: 'log line',
        offset: 100,
      });
      const result = await skillsApi.readRunLog('run-1');
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.skills_read_run_log',
          params: expect.objectContaining({ run_id: 'run-1' }),
        })
      );
      expect(result.bytes_read).toBe(100);
    });

    it('passes offset and max_bytes when provided', async () => {
      mockCallCoreRpc.mockResolvedValue({
        bytes_read: 0,
        eof: true,
        complete: true,
        content: '',
        offset: 500,
      });
      await skillsApi.readRunLog('run-2', 200, 4096);
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          params: expect.objectContaining({ run_id: 'run-2', offset: 200, max_bytes: 4096 }),
        })
      );
    });
  });

  describe('recentRuns', () => {
    it('returns scanned runs array', async () => {
      mockCallCoreRpc.mockResolvedValue({ runs: [] });
      const result = await skillsApi.recentRuns();
      expect(Array.isArray(result)).toBe(true);
    });

    it('passes skill_id filter when provided', async () => {
      mockCallCoreRpc.mockResolvedValue({ runs: [] });
      await skillsApi.recentRuns('dev-workflow', 5);
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          params: expect.objectContaining({ skill_id: 'dev-workflow', limit: 5 }),
        })
      );
    });
  });
});
