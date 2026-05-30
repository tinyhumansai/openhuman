import { createAsyncThunk, createSlice } from '@reduxjs/toolkit';
import debug from 'debug';

import { type Workflow, workflowsApi, type WorkflowSummary } from '../services/api/workflowsApi';
import { resetUserScopedState } from './resetActions';

const log = debug('workflows');

export type WorkflowsStatus = 'idle' | 'loading' | 'saving' | 'error';

export interface WorkflowsState {
  workflows: WorkflowSummary[];
  selectedWorkflow: Workflow | null;
  status: WorkflowsStatus;
  error: string | null;
}

const initialState: WorkflowsState = {
  workflows: [],
  selectedWorkflow: null,
  status: 'idle',
  error: null,
};

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}

export const loadWorkflows = createAsyncThunk('workflows/load', async () =>
  workflowsApi.listWorkflows()
);

export const readWorkflow = createAsyncThunk('workflows/read', async (id: string) =>
  workflowsApi.readWorkflow(id)
);

export const createWorkflow = createAsyncThunk(
  'workflows/create',
  async (params: { name: string; description?: string; when_to_use?: string }) =>
    workflowsApi.createWorkflow(params)
);

export const removeWorkflow = createAsyncThunk('workflows/remove', async (id: string) =>
  workflowsApi.uninstallWorkflow(id)
);

const workflowsSlice = createSlice({
  name: 'workflows',
  initialState,
  reducers: {
    clearSelectedWorkflow(state) {
      state.selectedWorkflow = null;
    },
  },
  extraReducers: builder => {
    builder
      // loadWorkflows
      .addCase(loadWorkflows.pending, state => {
        state.status = 'loading';
        state.error = null;
      })
      .addCase(loadWorkflows.fulfilled, (state, action) => {
        state.workflows = action.payload;
        state.status = 'idle';
        state.error = null;
        log('loaded %d workflow(s)', state.workflows.length);
      })
      .addCase(loadWorkflows.rejected, (state, action) => {
        state.status = 'error';
        state.error = errorMessage(action.error.message ?? action.error);
        log('load failed: %s', state.error);
      })

      // readWorkflow
      .addCase(readWorkflow.pending, state => {
        state.status = 'loading';
        state.error = null;
      })
      .addCase(readWorkflow.fulfilled, (state, action) => {
        state.selectedWorkflow = action.payload;
        state.status = 'idle';
        state.error = null;
        log('read workflow name=%s', action.payload.name);
      })
      .addCase(readWorkflow.rejected, (state, action) => {
        state.status = 'error';
        state.error = errorMessage(action.error.message ?? action.error);
      })

      // createWorkflow
      .addCase(createWorkflow.pending, state => {
        state.status = 'saving';
        state.error = null;
      })
      .addCase(createWorkflow.fulfilled, (state, action) => {
        const wf = action.payload;
        // Optimistically add to the list as a summary entry.
        const summary: WorkflowSummary = {
          id: wf.dir_name,
          name: wf.name,
          description: wf.description,
          when_to_use: wf.when_to_use,
          tags: wf.tags,
          scope: wf.scope,
          phases: Object.keys(wf.phases),
          warnings: wf.warnings,
        };
        const exists = state.workflows.some(w => w.id === summary.id);
        if (!exists) {
          state.workflows = [...state.workflows, summary];
        }
        state.selectedWorkflow = wf;
        state.status = 'idle';
        state.error = null;
        log('created workflow name=%s', wf.name);
      })
      .addCase(createWorkflow.rejected, (state, action) => {
        state.status = 'error';
        state.error = errorMessage(action.error.message ?? action.error);
      })

      // removeWorkflow
      .addCase(removeWorkflow.pending, state => {
        state.status = 'saving';
        state.error = null;
      })
      .addCase(removeWorkflow.fulfilled, (state, action) => {
        const removedId = action.payload.id;
        state.workflows = state.workflows.filter(w => w.id !== removedId);
        if (state.selectedWorkflow && state.selectedWorkflow.dir_name === removedId) {
          state.selectedWorkflow = null;
        }
        state.status = 'idle';
        state.error = null;
        log('removed workflow id=%s', removedId);
      })
      .addCase(removeWorkflow.rejected, (state, action) => {
        state.status = 'error';
        state.error = errorMessage(action.error.message ?? action.error);
      })

      .addCase(resetUserScopedState, () => initialState);
  },
});

export const { clearSelectedWorkflow } = workflowsSlice.actions;

export const selectWorkflows = (state: { workflows: WorkflowsState }) => state.workflows.workflows;

export const selectSelectedWorkflow = (state: { workflows: WorkflowsState }) =>
  state.workflows.selectedWorkflow;

export const selectWorkflowsStatus = (state: { workflows: WorkflowsState }) =>
  state.workflows.status;

export const selectWorkflowsError = (state: { workflows: WorkflowsState }) => state.workflows.error;

export default workflowsSlice.reducer;
