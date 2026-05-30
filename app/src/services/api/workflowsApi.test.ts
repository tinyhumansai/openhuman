import { beforeEach, describe, expect, it, vi } from 'vitest';

import { workflowsApi } from './workflowsApi';

const mockCallCoreRpc = vi.fn();
vi.mock('../coreRpcClient', () => ({ callCoreRpc: (...a: unknown[]) => mockCallCoreRpc(...a) }));

const mockWorkflow = {
  name: 'Test Workflow',
  dir_name: 'test-workflow',
  description: 'A test workflow',
  when_to_use: 'When testing',
  tags: ['test'],
  tools: null,
  phases: { on_pick_up_task: { rules: ['Follow TDD'], scripts: [], tools: null, context: [] } },
  location: '/home/user/.openhuman/workflows/test-workflow',
  scope: 'user' as const,
  warnings: [],
};

const mockSummary = {
  id: 'test-workflow',
  name: 'Test Workflow',
  description: 'A test workflow',
  when_to_use: 'When testing',
  tags: ['test'],
  scope: 'user' as const,
  phases: ['on_pick_up_task'],
  warnings: [],
};

describe('workflowsApi', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  describe('listWorkflows', () => {
    it('calls openhuman.workflows_list and returns workflows array', async () => {
      mockCallCoreRpc.mockResolvedValue({ workflows: [mockSummary] });
      const result = await workflowsApi.listWorkflows();
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({ method: 'openhuman.workflows_list' })
      );
      expect(result).toHaveLength(1);
      expect(result[0].id).toBe('test-workflow');
    });

    it('returns empty array when workflows is missing', async () => {
      mockCallCoreRpc.mockResolvedValue({});
      const result = await workflowsApi.listWorkflows();
      expect(result).toEqual([]);
    });

    it('unwraps data-envelope shape', async () => {
      mockCallCoreRpc.mockResolvedValue({ data: { workflows: [mockSummary] } });
      const result = await workflowsApi.listWorkflows();
      expect(result).toHaveLength(1);
    });
  });

  describe('readWorkflow', () => {
    it('calls openhuman.workflows_read with id param', async () => {
      mockCallCoreRpc.mockResolvedValue({ workflow: mockWorkflow });
      const result = await workflowsApi.readWorkflow('test-workflow');
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.workflows_read',
          params: { id: 'test-workflow' },
        })
      );
      expect(result.name).toBe('Test Workflow');
    });

    it('unwraps data-envelope shape', async () => {
      mockCallCoreRpc.mockResolvedValue({ data: { workflow: mockWorkflow } });
      const result = await workflowsApi.readWorkflow('test-workflow');
      expect(result.dir_name).toBe('test-workflow');
    });
  });

  describe('createWorkflow', () => {
    it('calls openhuman.workflows_create with name param', async () => {
      mockCallCoreRpc.mockResolvedValue({ workflow: mockWorkflow });
      const result = await workflowsApi.createWorkflow({ name: 'Test Workflow' });
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.workflows_create',
          params: expect.objectContaining({ name: 'Test Workflow' }),
        })
      );
      expect(result.name).toBe('Test Workflow');
    });

    it('includes optional description and when_to_use when provided', async () => {
      mockCallCoreRpc.mockResolvedValue({ workflow: mockWorkflow });
      await workflowsApi.createWorkflow({ name: 'Test', description: 'desc', when_to_use: 'when' });
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          params: expect.objectContaining({ description: 'desc', when_to_use: 'when' }),
        })
      );
    });

    it('omits undefined optional fields', async () => {
      mockCallCoreRpc.mockResolvedValue({ workflow: mockWorkflow });
      await workflowsApi.createWorkflow({ name: 'Test' });
      const call = mockCallCoreRpc.mock.calls[0][0] as { params: Record<string, unknown> };
      expect(call.params).not.toHaveProperty('description');
      expect(call.params).not.toHaveProperty('when_to_use');
    });
  });

  describe('uninstallWorkflow', () => {
    it('calls openhuman.workflows_uninstall with id param', async () => {
      mockCallCoreRpc.mockResolvedValue({ id: 'test-workflow', removed: true });
      const result = await workflowsApi.uninstallWorkflow('test-workflow');
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.workflows_uninstall',
          params: { id: 'test-workflow' },
        })
      );
      expect(result.removed).toBe(true);
    });

    it('unwraps data-envelope shape', async () => {
      mockCallCoreRpc.mockResolvedValue({ data: { id: 'test-workflow', removed: true } });
      const result = await workflowsApi.uninstallWorkflow('test-workflow');
      expect(result.id).toBe('test-workflow');
    });
  });

  describe('getWorkflowPhase', () => {
    it('calls openhuman.workflows_phase with id and phase params', async () => {
      mockCallCoreRpc.mockResolvedValue({ guidance: 'Follow TDD', tool_scope: null });
      const result = await workflowsApi.getWorkflowPhase('test-workflow', 'on_pick_up_task');
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.workflows_phase',
          params: { id: 'test-workflow', phase: 'on_pick_up_task' },
        })
      );
      expect(result.guidance).toBe('Follow TDD');
      expect(result.tool_scope).toBeNull();
    });

    it('handles null guidance', async () => {
      mockCallCoreRpc.mockResolvedValue({ guidance: null, tool_scope: null });
      const result = await workflowsApi.getWorkflowPhase('test-workflow', 'on_close_task');
      expect(result.guidance).toBeNull();
    });
  });
});
