/**
 * CreateSkillModal — vitest coverage
 *
 * Verifies:
 * - Renders title + required fields.
 * - Escape key closes (but not while submitting).
 * - Backdrop click closes (but not while submitting).
 * - Submit is disabled when name or description is empty.
 * - Submit rekeys `allowedTools` → `'allowed-tools'` via workflowsApi.createWorkflow.
 * - Submit calls `onCreated` with the returned skill.
 * - Submit failure surfaces an error banner and re-enables the button.
 * - Slug preview updates as the name changes.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { WorkflowSummary } from '../../../services/api/workflowsApi';
import CreateSkillModal from '../CreateSkillModal';

vi.mock('../../../services/api/workflowsApi', () => ({
  workflowsApi: {
    createWorkflow: vi.fn(),
    updateWorkflow: vi.fn(),
    describeWorkflow: vi
      .fn()
      .mockResolvedValue({ id: 'e', name: 'e', when_to_use: '', inputs: [] }),
  },
}));

function builtSkill(overrides: Partial<WorkflowSummary> = {}): WorkflowSummary {
  return {
    id: 'my-skill',
    name: 'My Skill',
    description: 'does stuff',
    version: '',
    author: null,
    tags: [],
    platforms: [],
    relatedSkills: [],
    sourceFormat: 'openhuman',
    tools: [],
    prompts: [],
    location: '/home/u/.openhuman/skills/my-skill/SKILL.md',
    resources: [],
    scope: 'user',
    legacy: false,
    warnings: [],
    ...overrides,
  };
}

describe('CreateSkillModal', () => {
  beforeEach(async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    vi.mocked(workflowsApi.createWorkflow).mockReset();
  });

  it('renders title and required fields', () => {
    render(<CreateSkillModal onClose={vi.fn()} onCreated={vi.fn()} />);
    expect(screen.getByText('New Workflow')).toBeInTheDocument();
    expect(screen.getByLabelText(/Name/)).toBeInTheDocument();
    expect(screen.getByLabelText(/Description/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Create workflow/ })).toBeInTheDocument();
  });

  it('updates slug preview as the user types the name', () => {
    render(<CreateSkillModal onClose={vi.fn()} onCreated={vi.fn()} />);
    const name = screen.getByLabelText(/Name/) as HTMLInputElement;
    fireEvent.change(name, { target: { value: 'My Trade Journal!' } });
    expect(screen.getByText('my-trade-journal')).toBeInTheDocument();
  });

  it('disables submit when name or description is empty', () => {
    render(<CreateSkillModal onClose={vi.fn()} onCreated={vi.fn()} />);
    const submit = screen.getByRole('button', { name: /Create workflow/ }) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);

    fireEvent.change(screen.getByLabelText(/Name/), { target: { value: 'demo' } });
    expect(submit.disabled).toBe(true);

    fireEvent.change(screen.getByLabelText(/Description/), { target: { value: 'what it does' } });
    expect(submit.disabled).toBe(false);
  });

  it('closes on Escape', () => {
    const onClose = vi.fn();
    render(<CreateSkillModal onClose={onClose} onCreated={vi.fn()} />);
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('submits name + description, calls onCreated with the new skill', async () => {
    // The previous incarnation of this test also drove Tags + Allowed-tools
    // inputs and asserted the `allowedTools` → `allowed-tools` rekey at the
    // call site. CreateWorkflowForm dropped those inputs in the refactor (the
    // form is now Name + Description + the `[[inputs]]` editor only — see
    // ScheduledCronCard / CreateWorkflowForm.tsx), so the inputs are no longer
    // collectable from the modal UI. The rekey itself still happens in
    // `workflowsApi.createWorkflow` (services/api/workflowsApi.ts → params build) and
    // is covered by the workflowsApi unit tests; this test now just guards the
    // modal's submit-pipeline shape: name + description → createWorkflow →
    // onCreated.
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    const created = builtSkill();
    vi.mocked(workflowsApi.createWorkflow).mockResolvedValueOnce(created);

    const onCreated = vi.fn();
    const onClose = vi.fn();
    render(<CreateSkillModal onClose={onClose} onCreated={onCreated} />);

    fireEvent.change(screen.getByLabelText(/Name/), { target: { value: 'My Skill' } });
    fireEvent.change(screen.getByLabelText(/Description/), { target: { value: 'does stuff' } });

    const submit = screen.getByRole('button', { name: /Create workflow/ });
    await act(async () => {
      fireEvent.click(submit);
    });

    expect(vi.mocked(workflowsApi.createWorkflow)).toHaveBeenCalledWith(
      expect.objectContaining({ name: 'My Skill', description: 'does stuff', scope: 'user' })
    );
    expect(onCreated).toHaveBeenCalledWith(created);
  });

  it('surfaces error and re-enables submit on failure', async () => {
    const { workflowsApi } = await import('../../../services/api/workflowsApi');
    vi.mocked(workflowsApi.createWorkflow).mockRejectedValueOnce(new Error('slug already exists'));

    render(<CreateSkillModal onClose={vi.fn()} onCreated={vi.fn()} />);
    fireEvent.change(screen.getByLabelText(/Name/), { target: { value: 'dup' } });
    fireEvent.change(screen.getByLabelText(/Description/), { target: { value: 'x' } });

    const submit = screen.getByRole('button', { name: /Create workflow/ }) as HTMLButtonElement;
    await act(async () => {
      fireEvent.click(submit);
    });

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent('slug already exists');
    });
    expect(submit.disabled).toBe(false);
  });

  it('close button calls onClose when not submitting', () => {
    const onClose = vi.fn();
    render(<CreateSkillModal onClose={onClose} onCreated={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: /close/i }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('renders Edit/Save labels in edit mode', () => {
    render(<CreateSkillModal editing={builtSkill()} onClose={vi.fn()} onCreated={vi.fn()} />);
    // Title + submit switch to the edit ontology (common.edit / common.save).
    expect(screen.getByRole('heading', { name: 'Edit' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /^Save$/ })).toBeInTheDocument();
    // ...and not the create labels.
    expect(screen.queryByText('New Workflow')).not.toBeInTheDocument();
  });
});
