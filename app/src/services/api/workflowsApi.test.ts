import { beforeEach, describe, expect, it, vi } from 'vitest';

import { workflowsApi } from './workflowsApi';

const mockCallCoreRpc = vi.fn();
vi.mock('../coreRpcClient', () => ({ callCoreRpc: (...a: unknown[]) => mockCallCoreRpc(...a) }));

describe('workflowsApi', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  describe('createWorkflow', () => {
    it('includes inputs in params when non-empty', async () => {
      mockCallCoreRpc.mockResolvedValue({
        workflow: { id: 's', name: 'S', description: '', scope: 'user' as const },
      });
      await workflowsApi.createWorkflow({
        name: 'S',
        description: 'desc',
        inputs: [{ name: 'repo', type: 'string' as const, description: 'repo', required: true }],
      });
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({ params: expect.objectContaining({ inputs: expect.any(Array) }) })
      );
    });
  });

  describe('describeWorkflow', () => {
    it('calls openhuman.workflows_describe with workflow_id', async () => {
      mockCallCoreRpc.mockResolvedValue({
        id: 'dev-workflow',
        name: 'Dev Workflow',
        description: 'Auto dev',
        inputs: [],
      });
      const result = await workflowsApi.describeWorkflow('dev-workflow');
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
      const result = await workflowsApi.describeWorkflow('x');
      expect(result.id).toBe('x');
    });
  });

  describe('runWorkflow', () => {
    it('calls openhuman.skill_runtime_run with skill_id and inputs', async () => {
      mockCallCoreRpc.mockResolvedValue({
        run_id: 'run-1',
        status: 'started',
        skill_id: 's',
        log: '/tmp/log',
      });
      const result = await workflowsApi.runWorkflow('s', { repo: 'owner/repo' });
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.skill_runtime_run',
          params: { skill_id: 's', inputs: { repo: 'owner/repo' } },
        })
      );
      expect(result.run_id).toBe('run-1');
      expect(result.workflow_id).toBe('s');
    });
  });

  describe('readRunLog', () => {
    it('calls skill_runtime_read_run_log with run_id', async () => {
      mockCallCoreRpc.mockResolvedValue({
        bytes_read: 100,
        eof: false,
        complete: false,
        content: 'log line',
        offset: 100,
      });
      const result = await workflowsApi.readRunLog('run-1');
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.skill_runtime_read_run_log',
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
      await workflowsApi.readRunLog('run-2', 200, 4096);
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
      const result = await workflowsApi.recentRuns();
      expect(Array.isArray(result)).toBe(true);
    });

    it('passes workflow_id filter when provided', async () => {
      mockCallCoreRpc.mockResolvedValue({ runs: [] });
      await workflowsApi.recentRuns('dev-workflow', 5);
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.skill_runtime_recent_runs',
          params: expect.objectContaining({ skill_id: 'dev-workflow', limit: 5 }),
        })
      );
    });
  });

  describe('createWorkflow (optional fields)', () => {
    it('forwards when_to_use, scope, license, author, tags, allowed-tools', async () => {
      mockCallCoreRpc.mockResolvedValue({
        workflow: { id: 's', name: 'S', description: '', scope: 'user' as const },
      });
      await workflowsApi.createWorkflow({
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
        workflow: { id: 's', name: 'S', description: '', scope: 'user' as const },
      });
      await workflowsApi.createWorkflow({ name: 'S', description: 'd', whenToUse: '   ' });
      const params = mockCallCoreRpc.mock.calls[0][0].params;
      expect(params).not.toHaveProperty('when_to_use');
    });
  });

  describe('updateWorkflow', () => {
    it('calls openhuman.workflows_update and returns the skill', async () => {
      mockCallCoreRpc.mockResolvedValue({
        workflow: { id: 'wf', name: 'WF', description: 'd', scope: 'user' as const },
      });
      const result = await workflowsApi.updateWorkflow({
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
        data: { workflow: { id: 'wf2', name: 'WF2', description: '', scope: 'user' as const } },
      });
      const result = await workflowsApi.updateWorkflow({ name: 'WF2', description: 'd' });
      expect(result.id).toBe('wf2');
    });
  });

  describe('cancelRun', () => {
    it('calls openhuman.skill_runtime_cancel with run_id and returns cancelled', async () => {
      mockCallCoreRpc.mockResolvedValue({ cancelled: true });
      const result = await workflowsApi.cancelRun('run-9');
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.skill_runtime_cancel',
          params: { run_id: 'run-9' },
        })
      );
      expect(result).toBe(true);
    });

    it('returns false when the run was not live (envelope shape)', async () => {
      mockCallCoreRpc.mockResolvedValue({ data: { cancelled: false } });
      const result = await workflowsApi.cancelRun('gone');
      expect(result).toBe(false);
    });
  });

  describe('uninstallWorkflow', () => {
    it('calls openhuman.skill_registry_uninstall and normalizes removed_path', async () => {
      mockCallCoreRpc.mockResolvedValue({ name: 'demo', removed_path: '/tmp/demo', scope: 'user' });
      const result = await workflowsApi.uninstallWorkflow('demo');
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.skill_registry_uninstall',
          params: { name: 'demo' },
        })
      );
      expect(result.removedPath).toBe('/tmp/demo');
    });
  });

  describe('resolveRuntimes', () => {
    it('calls openhuman.skill_runtime_resolve_runtimes and normalizes bin_dir', async () => {
      mockCallCoreRpc.mockResolvedValue({
        runtimes: [
          {
            runtime: 'node',
            enabled: true,
            available: true,
            source: 'system',
            version: '22.11.0',
            binary: '/usr/bin/node',
            bin_dir: '/usr/bin',
            error: null,
          },
        ],
      });
      const result = await workflowsApi.resolveRuntimes('node');
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.skill_runtime_resolve_runtimes',
          params: { runtime: 'node' },
        })
      );
      expect(result.runtimes[0].binDir).toBe('/usr/bin');
    });

    it('uses empty params object when runtime is "all" (the default)', async () => {
      mockCallCoreRpc.mockResolvedValue({ runtimes: [] });

      await workflowsApi.resolveRuntimes('all');

      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({ method: 'openhuman.skill_runtime_resolve_runtimes', params: {} })
      );
    });

    it('uses empty params when called with no argument (default = all)', async () => {
      mockCallCoreRpc.mockResolvedValue({ runtimes: [] });

      await workflowsApi.resolveRuntimes();

      const call = mockCallCoreRpc.mock.calls[0][0];
      expect(call.params).toEqual({});
    });
  });
});
