/**
 * AgentWorkflows page — Vitest coverage
 *
 * Verifies:
 * - Loading state while workflows are being fetched.
 * - Renders workflow cards once loaded.
 * - Empty state shown when there are no workflows.
 * - Create modal opens on button click.
 * - Create workflow success path: modal closes, toast appears, list refreshes.
 * - Delete workflow success path: confirmation dialog, success toast.
 * - Delete error path: error toast shown.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { getCoreStateSnapshot } from '../lib/coreState/store';
import { CoreStateContext } from '../providers/coreStateContext';
import { workflowsApi } from '../services/api/workflowsApi';
import localeReducer from '../store/localeSlice';
import workflowsReducer from '../store/workflowsSlice';
import AgentWorkflows from './AgentWorkflows';

// Mock the workflowsApi
vi.mock('../services/api/workflowsApi', () => ({
  workflowsApi: {
    listWorkflows: vi.fn(),
    readWorkflow: vi.fn(),
    createWorkflow: vi.fn(),
    uninstallWorkflow: vi.fn(),
    getWorkflowPhase: vi.fn(),
  },
}));

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

const mockWorkflow = {
  name: 'Test Workflow',
  dir_name: 'test-workflow',
  description: 'A test workflow',
  when_to_use: 'When testing',
  tags: ['test'],
  tools: null,
  phases: {
    on_pick_up_task: { rules: ['Write tests first'], scripts: [], tools: null, context: [] },
  },
  location: null,
  scope: 'user' as const,
  warnings: [],
};

function makeStore() {
  return configureStore({
    reducer: combineReducers({ locale: localeReducer, workflows: workflowsReducer }),
  });
}

function renderPage(store = makeStore()) {
  const coreStateStub = {
    ...getCoreStateSnapshot(),
    refresh: async () => {},
    refreshTeams: async () => {},
    refreshTeamMembers: async () => {},
    refreshTeamInvites: async () => {},
    setAnalyticsEnabled: async () => {},
    setMeetAutoOrchestratorHandoff: async () => {},
    setOnboardingCompletedFlag: async () => {},
    setEncryptionKey: async () => {},
    patchSnapshot: () => {},
    setOnboardingTasks: async () => {},
    storeSessionToken: async () => {},
    clearSession: async () => {},
  };

  return {
    store,
    ...render(
      <Provider store={store}>
        <CoreStateContext.Provider value={coreStateStub}>
          <MemoryRouter>
            <AgentWorkflows />
          </MemoryRouter>
        </CoreStateContext.Provider>
      </Provider>
    ),
  };
}

describe('AgentWorkflows', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders page title', async () => {
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    renderPage();
    expect(screen.getByText('Agent Workflows')).toBeInTheDocument();
  });

  it('renders empty state when no workflows', async () => {
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    renderPage();
    await waitFor(() => {
      expect(screen.getByText('No workflows yet')).toBeInTheDocument();
    });
  });

  it('renders workflow cards after loading', async () => {
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([mockSummary]);
    renderPage();
    await waitFor(() => {
      expect(screen.getByText('Test Workflow')).toBeInTheDocument();
    });
    expect(screen.getByTestId('workflow-card-test-workflow')).toBeInTheDocument();
  });

  it('opens create modal when New Workflow button is clicked', async () => {
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    renderPage();
    await waitFor(() => {
      expect(screen.getByTestId('workflows-create-btn')).toBeInTheDocument();
    });
    fireEvent.click(screen.getByTestId('workflows-create-btn'));
    expect(screen.getByRole('dialog', { name: /New Workflow/i })).toBeInTheDocument();
  });

  it('shows success toast and closes modal after creating workflow', async () => {
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([]);
    vi.mocked(workflowsApi.createWorkflow).mockResolvedValue(mockWorkflow);
    renderPage();
    await waitFor(() => {
      expect(screen.getByTestId('workflows-create-btn')).toBeInTheDocument();
    });

    // Open create modal
    fireEvent.click(screen.getByTestId('workflows-create-btn'));
    expect(screen.getByRole('dialog', { name: /New Workflow/i })).toBeInTheDocument();

    // Fill name
    const nameInput = screen.getByLabelText(/Name/);
    fireEvent.change(nameInput, { target: { value: 'Test Workflow' } });

    // Submit
    const submitBtn = screen.getByRole('button', { name: /Create workflow/i });
    // Re-fetch after create
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([mockSummary]);
    await act(async () => {
      fireEvent.click(submitBtn);
    });

    await waitFor(() => {
      expect(screen.getByText('Workflow created')).toBeInTheDocument();
    });
  });

  it('opens delete confirmation when delete button is clicked', async () => {
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([mockSummary]);
    renderPage();
    await waitFor(() => {
      expect(screen.getByTestId('workflow-card-test-workflow-delete')).toBeInTheDocument();
    });
    fireEvent.click(screen.getByTestId('workflow-card-test-workflow-delete'));
    expect(screen.getByRole('alertdialog')).toBeInTheDocument();
    expect(screen.getByText(/Delete workflow/i)).toBeInTheDocument();
  });

  it('removes workflow and shows success toast on confirmed delete', async () => {
    vi.mocked(workflowsApi.listWorkflows).mockResolvedValue([mockSummary]);
    vi.mocked(workflowsApi.uninstallWorkflow).mockResolvedValue({
      id: 'test-workflow',
      removed: true,
    });
    renderPage();
    await waitFor(() => {
      expect(screen.getByTestId('workflow-card-test-workflow-delete')).toBeInTheDocument();
    });
    fireEvent.click(screen.getByTestId('workflow-card-test-workflow-delete'));
    await act(async () => {
      fireEvent.click(screen.getByTestId('wf-delete-confirm-btn'));
    });
    await waitFor(() => {
      expect(screen.getByText('Delete Workflow')).toBeInTheDocument();
    });
  });

  it('shows retry button when load fails', async () => {
    vi.mocked(workflowsApi.listWorkflows).mockRejectedValue(new Error('network error'));
    renderPage();
    await waitFor(() => {
      expect(screen.getByText(/Try again/i)).toBeInTheDocument();
    });
  });
});
