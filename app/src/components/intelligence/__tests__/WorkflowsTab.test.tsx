import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { WorkflowSummary } from '../../../services/api/workflowsApi';
import WorkflowsTab from '../WorkflowsTab';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

// Stable navigate spy so we can assert the runner deep-links.
const navigate = vi.fn();
vi.mock('react-router-dom', () => ({ useNavigate: () => navigate }));

// Stub the create modal + delete dialog so we can drive WorkflowsTab's own
// onCreated / onUninstalled handlers (navigation, list reconcile) without
// pulling in their internals.
vi.mock('../../skills/CreateSkillModal', () => ({
  default: ({
    onCreated,
    onClose,
  }: {
    onCreated: (wf: WorkflowSummary) => void;
    onClose: () => void;
  }) => (
    <div data-testid="create-modal-stub">
      <button
        data-testid="stub-create"
        onClick={() =>
          onCreated({
            id: 'new-wf',
            name: 'New WF',
            description: '',
            version: '',
            author: null,
            tags: [],
            platforms: [],
            relatedSkills: [],
            sourceFormat: 'openhuman',
            tools: [],
            prompts: [],
            location: null,
            resources: [],
            scope: 'user',
            legacy: false,
            warnings: [],
          })
        }
      />
      <button data-testid="stub-create-close" onClick={onClose} />
    </div>
  ),
}));
vi.mock('../../skills/UninstallSkillConfirmDialog', () => ({
  default: ({
    onUninstalled,
  }: {
    onUninstalled: (r: { name: string; removedPath: string; scope: string }) => void;
  }) => (
    <button
      data-testid="stub-uninstalled"
      onClick={() => onUninstalled({ name: 'display-name', removedPath: '/x', scope: 'user' })}
    />
  ),
}));

const seeded = (overrides: Partial<WorkflowSummary>): WorkflowSummary => ({
  id: 'wf-1',
  name: 'WF 1',
  description: 'A workflow.',
  version: '0.1.0',
  author: null,
  tags: [],
  platforms: [],
  relatedSkills: [],
  sourceFormat: 'openhuman',
  tools: [],
  prompts: [],
  location: null,
  resources: [],
  scope: 'user',
  legacy: false,
  warnings: [],
  ...overrides,
});

const listWorkflows = vi.fn();
vi.mock('../../../services/api/workflowsApi', async () => {
  const actual = await vi.importActual<typeof import('../../../services/api/workflowsApi')>(
    '../../../services/api/workflowsApi'
  );
  return {
    ...actual,
    workflowsApi: { ...actual.workflowsApi, listWorkflows: () => listWorkflows() },
  };
});

describe('WorkflowsTab', () => {
  beforeEach(() => {
    navigate.mockReset();
    listWorkflows.mockReset();
  });

  it('lists workflows from workflowsApi with the create entry point', async () => {
    listWorkflows.mockResolvedValue([
      seeded({ id: 'user-wf', name: 'User WF', scope: 'user' }),
      seeded({ id: 'project-wf', name: 'Project WF', scope: 'project' }),
    ]);
    render(<WorkflowsTab />);

    await waitFor(() => expect(screen.getByText('User WF')).toBeInTheDocument());
    expect(screen.getByText('Project WF')).toBeInTheDocument();
    expect(screen.getByTestId('workflows-list')).toBeInTheDocument();
    expect(screen.getByTestId('workflow-card-user-wf')).toBeInTheDocument();

    // Create entry point lives here now (not on Connections). The
    // install-from-URL button was retired; authoring is create-only.
    expect(screen.getByTestId('workflows-create-btn')).toBeInTheDocument();
    expect(screen.queryByTestId('workflows-install-btn')).not.toBeInTheDocument();
  });

  it('navigates to the locked runner page when a card is opened', async () => {
    listWorkflows.mockResolvedValue([seeded({ id: 'user-wf', name: 'User WF', scope: 'user' })]);
    render(<WorkflowsTab />);
    await waitFor(() => expect(screen.getByTestId('workflow-open-user-wf')).toBeInTheDocument());

    fireEvent.click(screen.getByTestId('workflow-open-user-wf'));
    expect(navigate).toHaveBeenCalledWith('/workflows/run?workflow=user-wf&lock=1');
  });

  it('opens create and lands on the new workflow runner after create', async () => {
    listWorkflows.mockResolvedValue([]);
    render(<WorkflowsTab />);
    await waitFor(() => expect(screen.getByTestId('workflows-create-btn')).toBeInTheDocument());

    fireEvent.click(screen.getByTestId('workflows-create-btn'));
    fireEvent.click(screen.getByTestId('stub-create'));
    expect(navigate).toHaveBeenCalledWith('/workflows/run?workflow=new-wf&lock=1');
  });

  it('shows an error panel with retry when listWorkflows fails (not the empty state)', async () => {
    listWorkflows.mockRejectedValueOnce(new Error('backend down'));
    render(<WorkflowsTab />);

    await waitFor(() => expect(screen.getByTestId('workflows-load-error')).toBeInTheDocument());
    expect(screen.getByText('backend down')).toBeInTheDocument();
    expect(screen.queryByText('workflows.empty.title')).not.toBeInTheDocument();

    // Retry re-fetches and renders the list.
    listWorkflows.mockResolvedValueOnce([
      seeded({ id: 'user-wf', name: 'User WF', scope: 'user' }),
    ]);
    fireEvent.click(screen.getByText('common.retry'));
    await waitFor(() => expect(screen.getByText('User WF')).toBeInTheDocument());
  });

  it('removes the workflow from the list after uninstall, keyed by id', async () => {
    listWorkflows.mockResolvedValue([seeded({ id: 'user-wf', name: 'User WF', scope: 'user' })]);
    render(<WorkflowsTab />);
    await waitFor(() => expect(screen.getByTestId('workflow-card-user-wf')).toBeInTheDocument());

    // Open the card's "More actions" menu, then the delete action.
    fireEvent.click(screen.getByTitle('skills.card.moreActions'));
    fireEvent.click(screen.getByTestId('workflow-uninstall-user-wf'));
    // Stubbed dialog confirms the uninstall.
    fireEvent.click(screen.getByTestId('stub-uninstalled'));

    await waitFor(() => expect(screen.queryByText('User WF')).not.toBeInTheDocument());
  });

  it('renders the empty state when no workflows are installed', async () => {
    listWorkflows.mockResolvedValue([]);
    render(<WorkflowsTab />);
    await waitFor(() => expect(screen.getByText('workflows.empty.title')).toBeInTheDocument());
    expect(screen.queryByTestId('workflows-list')).not.toBeInTheDocument();
  });
});
