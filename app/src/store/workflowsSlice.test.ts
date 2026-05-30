import { configureStore } from '@reduxjs/toolkit';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { workflowsApi } from '../services/api/workflowsApi';
import workflowsReducer, {
  clearSelectedWorkflow,
  createWorkflow,
  loadWorkflows,
  readWorkflow,
  removeWorkflow,
  selectSelectedWorkflow,
  selectWorkflows,
  selectWorkflowsError,
  selectWorkflowsStatus,
  type WorkflowsState,
} from './workflowsSlice';

// Mock the workflowsApi
vi.mock('../services/api/workflowsApi', () => ({
  workflowsApi: {
    listWorkflows: vi.fn(),
    readWorkflow: vi.fn(),
    createWorkflow: vi.fn(),
    uninstallWorkflow: vi.fn(),
  },
}));

const mockWorkflow = {
  name: 'Test Workflow',
  dir_name: 'test-workflow',
  description: 'A test',
  when_to_use: 'When testing',
  tags: [],
  tools: null,
  phases: {
    on_pick_up_task: { rules: ['Write tests first'], scripts: [], tools: null, context: [] },
  },
  location: null,
  scope: 'user' as const,
  warnings: [],
};

const mockSummary = {
  id: 'test-workflow',
  name: 'Test Workflow',
  description: 'A test',
  when_to_use: 'When testing',
  tags: [],
  scope: 'user' as const,
  phases: ['on_pick_up_task'],
  warnings: [],
};

function makeStore() {
  return configureStore({ reducer: { workflows: workflowsReducer } });
}

describe('workflowsSlice', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('has correct initial state', () => {
    const store = makeStore();
    const state = store.getState().workflows as WorkflowsState;
    expect(state.workflows).toEqual([]);
    expect(state.selectedWorkflow).toBeNull();
    expect(state.status).toBe('idle');
    expect(state.error).toBeNull();
  });

  describe('loadWorkflows', () => {
    it('sets status to loading on pending', () => {
      const store = makeStore();
      // dispatch without resolving
      vi.mocked(workflowsApi.listWorkflows).mockReturnValue(new Promise(() => {}));
      void store.dispatch(loadWorkflows());
      const state = store.getState().workflows as WorkflowsState;
      expect(state.status).toBe('loading');
      expect(state.error).toBeNull();
    });

    it('populates workflows on fulfilled', async () => {
      const store = makeStore();
      vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([mockSummary]);
      await store.dispatch(loadWorkflows());
      const state = store.getState().workflows as WorkflowsState;
      expect(state.workflows).toHaveLength(1);
      expect(state.workflows[0].id).toBe('test-workflow');
      expect(state.status).toBe('idle');
      expect(state.error).toBeNull();
    });

    it('sets error on rejected', async () => {
      const store = makeStore();
      vi.mocked(workflowsApi.listWorkflows).mockRejectedValue(new Error('RPC failed'));
      await store.dispatch(loadWorkflows());
      const state = store.getState().workflows as WorkflowsState;
      expect(state.status).toBe('error');
      expect(state.error).toContain('RPC failed');
    });
  });

  describe('readWorkflow', () => {
    it('sets selectedWorkflow on fulfilled', async () => {
      const store = makeStore();
      vi.mocked(workflowsApi.readWorkflow).mockResolvedValue(mockWorkflow);
      await store.dispatch(readWorkflow('test-workflow'));
      const state = store.getState().workflows as WorkflowsState;
      expect(state.selectedWorkflow).toEqual(mockWorkflow);
      expect(state.status).toBe('idle');
    });
  });

  describe('createWorkflow', () => {
    it('adds workflow to list and sets selectedWorkflow on fulfilled', async () => {
      const store = makeStore();
      vi.mocked(workflowsApi.createWorkflow).mockResolvedValue(mockWorkflow);
      await store.dispatch(createWorkflow({ name: 'Test Workflow' }));
      const state = store.getState().workflows as WorkflowsState;
      expect(state.workflows).toHaveLength(1);
      expect(state.workflows[0].id).toBe('test-workflow');
      expect(state.selectedWorkflow).toEqual(mockWorkflow);
      expect(state.status).toBe('idle');
    });

    it('does not duplicate if workflow id already exists', async () => {
      const store = makeStore();
      vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([mockSummary]);
      await store.dispatch(loadWorkflows());
      vi.mocked(workflowsApi.createWorkflow).mockResolvedValue(mockWorkflow);
      await store.dispatch(createWorkflow({ name: 'Test Workflow' }));
      const state = store.getState().workflows as WorkflowsState;
      expect(state.workflows).toHaveLength(1);
    });

    it('sets status to saving on pending', () => {
      const store = makeStore();
      vi.mocked(workflowsApi.createWorkflow).mockReturnValue(new Promise(() => {}));
      void store.dispatch(createWorkflow({ name: 'Test' }));
      const state = store.getState().workflows as WorkflowsState;
      expect(state.status).toBe('saving');
    });
  });

  describe('removeWorkflow', () => {
    it('removes workflow from list on fulfilled', async () => {
      const store = makeStore();
      vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([mockSummary]);
      await store.dispatch(loadWorkflows());
      vi.mocked(workflowsApi.uninstallWorkflow).mockResolvedValue({
        id: 'test-workflow',
        removed: true,
      });
      await store.dispatch(removeWorkflow('test-workflow'));
      const state = store.getState().workflows as WorkflowsState;
      expect(state.workflows).toHaveLength(0);
      expect(state.status).toBe('idle');
    });

    it('clears selectedWorkflow if it was the removed workflow', async () => {
      const store = makeStore();
      vi.mocked(workflowsApi.readWorkflow).mockResolvedValue(mockWorkflow);
      await store.dispatch(readWorkflow('test-workflow'));
      vi.mocked(workflowsApi.uninstallWorkflow).mockResolvedValue({
        id: 'test-workflow',
        removed: true,
      });
      await store.dispatch(removeWorkflow('test-workflow'));
      const state = store.getState().workflows as WorkflowsState;
      expect(state.selectedWorkflow).toBeNull();
    });
  });

  describe('clearSelectedWorkflow', () => {
    it('sets selectedWorkflow to null', async () => {
      const store = makeStore();
      vi.mocked(workflowsApi.readWorkflow).mockResolvedValue(mockWorkflow);
      await store.dispatch(readWorkflow('test-workflow'));
      store.dispatch(clearSelectedWorkflow());
      const state = store.getState().workflows as WorkflowsState;
      expect(state.selectedWorkflow).toBeNull();
    });
  });

  describe('selectors', () => {
    it('selectWorkflows returns the workflows array', async () => {
      const store = makeStore();
      vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([mockSummary]);
      await store.dispatch(loadWorkflows());
      const workflows = selectWorkflows(store.getState() as { workflows: WorkflowsState });
      expect(workflows).toHaveLength(1);
    });

    it('selectSelectedWorkflow returns null initially', () => {
      const store = makeStore();
      const selected = selectSelectedWorkflow(store.getState() as { workflows: WorkflowsState });
      expect(selected).toBeNull();
    });

    it('selectWorkflowsStatus returns idle initially', () => {
      const store = makeStore();
      const s = selectWorkflowsStatus(store.getState() as { workflows: WorkflowsState });
      expect(s).toBe('idle');
    });

    it('selectWorkflowsError returns null initially', () => {
      const store = makeStore();
      const err = selectWorkflowsError(store.getState() as { workflows: WorkflowsState });
      expect(err).toBeNull();
    });
  });
});
