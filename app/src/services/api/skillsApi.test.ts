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
    it('calls openhuman.workflows_describe with workflow_id', async () => {
      mockCallCoreRpc.mockResolvedValue({
        id: 'dev-workflow',
        name: 'Dev Workflow',
        description: 'Auto dev',
        inputs: [],
      });
      const result = await skillsApi.describeSkill('dev-workflow');
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.workflows_describe',
          params: { workflow_id: 'dev-workflow' },
        })
      );
      expect(result.id).toBe('dev-workflow');
    });

    it('unwraps data-envelope shape', async () => {
      mockCallCoreRpc.mockResolvedValue({
        data: { id: 'x', name: 'X', description: '', inputs: [], workflow_id: 'x' },
      });
      const result = await skillsApi.describeSkill('x');
      expect(result.id).toBe('x');
    });
  });

  describe('runSkill', () => {
    it('calls openhuman.workflows_run with workflow_id and inputs', async () => {
      mockCallCoreRpc.mockResolvedValue({ run_id: 'run-1', workflow_id: 's', log: '/tmp/log' });
      const result = await skillsApi.runSkill('s', { repo: 'owner/repo' });
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.workflows_run',
          params: { workflow_id: 's', inputs: { repo: 'owner/repo' } },
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
          method: 'openhuman.workflows_read_run_log',
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

    it('passes workflow_id filter when provided', async () => {
      mockCallCoreRpc.mockResolvedValue({ runs: [] });
      await skillsApi.recentRuns('dev-workflow', 5);
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          params: expect.objectContaining({ workflow_id: 'dev-workflow', limit: 5 }),
        })
      );
    });
  });

  describe('createSkill (optional fields)', () => {
    it('forwards when_to_use, scope, license, author, tags, allowed-tools', async () => {
      mockCallCoreRpc.mockResolvedValue({
        skill: { id: 's', name: 'S', description: '', scope: 'user' as const },
      });
      await skillsApi.createSkill({
        name: 'S',
        description: 'desc',
        whenToUse: 'when asked',
        scope: 'user',
        license: 'MIT',
        author: 'me',
        tags: ['a'],
        allowedTools: ['shell'],
      });
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.workflows_create',
          params: expect.objectContaining({
            when_to_use: 'when asked',
            scope: 'user',
            license: 'MIT',
            author: 'me',
            tags: ['a'],
            'allowed-tools': ['shell'],
          }),
        })
      );
    });

    it('omits when_to_use when blank', async () => {
      mockCallCoreRpc.mockResolvedValue({
        skill: { id: 's', name: 'S', description: '', scope: 'user' as const },
      });
      await skillsApi.createSkill({ name: 'S', description: 'd', whenToUse: '   ' });
      const params = mockCallCoreRpc.mock.calls[0][0].params;
      expect(params).not.toHaveProperty('when_to_use');
    });
  });

  describe('updateSkill', () => {
    it('calls openhuman.workflows_update and returns the skill', async () => {
      mockCallCoreRpc.mockResolvedValue({
        skill: { id: 'wf', name: 'WF', description: 'd', scope: 'user' as const },
      });
      const result = await skillsApi.updateSkill({
        name: 'WF',
        description: 'd',
        whenToUse: 'edit trigger',
        inputs: [{ name: 'x', type: 'string' as const, description: 'x', required: false }],
      });
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.workflows_update',
          params: expect.objectContaining({
            name: 'WF',
            when_to_use: 'edit trigger',
            inputs: expect.any(Array),
          }),
        })
      );
      expect(result.id).toBe('wf');
    });

    it('unwraps the data-envelope shape', async () => {
      mockCallCoreRpc.mockResolvedValue({
        data: { skill: { id: 'wf2', name: 'WF2', description: '', scope: 'user' as const } },
      });
      const result = await skillsApi.updateSkill({ name: 'WF2', description: 'd' });
      expect(result.id).toBe('wf2');
    });
  });

  describe('cancelRun', () => {
    it('calls openhuman.workflows_cancel with run_id and returns cancelled', async () => {
      mockCallCoreRpc.mockResolvedValue({ cancelled: true });
      const result = await skillsApi.cancelRun('run-9');
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.workflows_cancel',
          params: { run_id: 'run-9' },
        })
      );
      expect(result).toBe(true);
    });

    it('returns false when the run was not live (envelope shape)', async () => {
      mockCallCoreRpc.mockResolvedValue({ data: { cancelled: false } });
      const result = await skillsApi.cancelRun('gone');
      expect(result).toBe(false);
    });
  });
});
