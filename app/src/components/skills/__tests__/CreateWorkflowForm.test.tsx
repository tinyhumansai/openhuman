/**
 * CreateWorkflowForm — standalone form coverage.
 *
 * Phase 5 of the /skills IA restructure: validates the form behaves
 * the same way as it did inside CreateSkillModal, so both the modal
 * and the /skills/new page can rely on it.
 *
 * Covers:
 *  - submit calls workflowsApi.createWorkflow with the trimmed/normalised
 *    payload (CSVs split, optional fields omitted when empty).
 *  - onStateChange is called with validity + submitting flags so
 *    wrappers can sync their submit button's disabled state.
 *  - error path surfaces the Rust message in role="alert".
 *  - slug preview reflects the typed name.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import CreateWorkflowForm, { previewSlug } from '../CreateWorkflowForm';

const stableT = (key: string) => key;
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: stableT }) }));

const hoisted = vi.hoisted(() => ({
  createWorkflow: vi.fn(),
  updateWorkflow: vi.fn(),
  describeWorkflow: vi.fn(),
}));

vi.mock('../../../services/api/workflowsApi', () => ({
  workflowsApi: {
    createWorkflow: hoisted.createWorkflow,
    updateWorkflow: hoisted.updateWorkflow,
    describeWorkflow: hoisted.describeWorkflow,
  },
}));

const FORM_ID = 'create-skill-test-form';

describe('previewSlug', () => {
  it('lowercases ASCII alnum, collapses spaces/underscores to single hyphens', () => {
    expect(previewSlug('My New Skill')).toBe('my-new-skill');
    expect(previewSlug('foo___bar')).toBe('foo-bar');
    expect(previewSlug('Hello, World!')).toBe('hello-world');
  });

  it('trims leading/trailing hyphens', () => {
    expect(previewSlug('  - leading and trailing - ')).toBe('leading-and-trailing');
  });

  it('strips diacritics via NFKD and drops symbols', () => {
    // NFKD decomposes é → e + combining acute; the combining mark is
    // outside ASCII alnum so it's dropped, leaving `cafe-beans`.
    expect(previewSlug('café & beans')).toBe('cafe-beans');
  });
});

describe('CreateWorkflowForm', () => {
  beforeEach(() => {
    hoisted.createWorkflow.mockReset();
  });

  it('renders required fields and the slug preview updates as the name changes', () => {
    render(<CreateWorkflowForm formId={FORM_ID} onCreated={vi.fn()} />);

    const nameInput = screen.getByLabelText(/skills.create.name/i) as HTMLInputElement;
    fireEvent.change(nameInput, { target: { value: 'My Cool Skill' } });
    expect(screen.getByText('my-cool-skill')).toBeInTheDocument();
  });

  it('reports validity to the wrapper via onStateChange when name and description are filled', () => {
    const onStateChange = vi.fn();
    render(
      <CreateWorkflowForm formId={FORM_ID} onCreated={vi.fn()} onStateChange={onStateChange} />
    );

    // Initially invalid (empty form).
    expect(onStateChange).toHaveBeenLastCalledWith({ valid: false, submitting: false });

    fireEvent.change(screen.getByLabelText(/skills.create.name/i), {
      target: { value: 'My Skill' },
    });
    // Name alone is not enough.
    expect(onStateChange).toHaveBeenLastCalledWith({ valid: false, submitting: false });

    fireEvent.change(screen.getByLabelText(/skills.create.description/i), {
      target: { value: 'Does the thing.' },
    });
    expect(onStateChange).toHaveBeenLastCalledWith({ valid: true, submitting: false });
  });

  it('submits the trimmed minimal payload (name + description + scope=user)', async () => {
    // The form was simplified to name + description only — scope is
    // hard-coded to 'user' (the only sensible default for skills created
    // through the UI), and the previous license/author/tags/allowed-tools
    // fields were dropped. Anyone needing project-scoped or
    // tagged skills edits the workspace SKILL.md directly.
    const created = { id: 'my-skill', name: 'My Skill', scope: 'user', legacy: false };
    hoisted.createWorkflow.mockResolvedValue(created);
    const onCreated = vi.fn();

    render(<CreateWorkflowForm formId={FORM_ID} onCreated={onCreated} />);

    fireEvent.change(screen.getByLabelText(/skills.create.name/i), {
      target: { value: '  My Skill  ' },
    });
    fireEvent.change(screen.getByLabelText(/skills.create.description/i), {
      target: { value: '  Does the thing.  ' },
    });

    // The form has no internal submit button — fire a submit event on
    // the <form id> directly (this is what `<button form=...>` does
    // from a wrapper).
    fireEvent.submit(document.getElementById(FORM_ID)!);

    await waitFor(() => {
      expect(hoisted.createWorkflow).toHaveBeenCalledWith({
        name: 'My Skill',
        description: 'Does the thing.',
        scope: 'user',
      });
    });
    expect(onCreated).toHaveBeenCalledWith(created);
  });

  it('includes whenToUse in the payload when the trigger field is filled', async () => {
    hoisted.createWorkflow.mockResolvedValue({
      id: 'wf',
      name: 'wf',
      scope: 'user',
      legacy: false,
    });
    render(<CreateWorkflowForm formId={FORM_ID} onCreated={vi.fn()} />);

    fireEvent.change(screen.getByLabelText(/skills.create.name/i), { target: { value: 'Triage' } });
    fireEvent.change(screen.getByLabelText(/skills.create.description/i), {
      target: { value: 'Summarise the inbox.' },
    });
    fireEvent.change(screen.getByLabelText(/skills.create.whenToUse/i), {
      target: { value: '  when the user asks to triage email  ' },
    });
    fireEvent.submit(document.getElementById(FORM_ID)!);

    await waitFor(() => {
      expect(hoisted.createWorkflow).toHaveBeenCalledWith({
        name: 'Triage',
        description: 'Summarise the inbox.',
        scope: 'user',
        whenToUse: 'when the user asks to triage email',
      });
    });
  });

  it('surfaces the Rust error message in an alert when createWorkflow rejects', async () => {
    hoisted.createWorkflow.mockRejectedValue(new Error('slug already exists'));
    render(<CreateWorkflowForm formId={FORM_ID} onCreated={vi.fn()} />);

    fireEvent.change(screen.getByLabelText(/skills.create.name/i), { target: { value: 'Dupe' } });
    fireEvent.change(screen.getByLabelText(/skills.create.description/i), {
      target: { value: 'whatever' },
    });
    fireEvent.submit(document.getElementById(FORM_ID)!);

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('slug already exists');
  });

  it('does not call createWorkflow if the form is invalid (no name)', async () => {
    render(<CreateWorkflowForm formId={FORM_ID} onCreated={vi.fn()} />);
    fireEvent.submit(document.getElementById(FORM_ID)!);
    // Give the microtask queue a tick — should still be 0.
    await Promise.resolve();
    expect(hoisted.createWorkflow).not.toHaveBeenCalled();
  });

  // ── Inputs editor ───────────────────────────────────────────────────
  // The form gained an optional [[inputs]] editor in 5d77839f. These
  // tests pin its contract end-to-end: the rows the user adds become the
  // `inputs` field on the createWorkflow payload, name validation blocks
  // submission, and removing a row drops it from the payload.

  /** Fill name + description so the rest of the form is submittable. */
  function fillRequiredFields() {
    fireEvent.change(screen.getByLabelText(/skills.create.name/i), {
      target: { value: 'My Skill' },
    });
    fireEvent.change(screen.getByLabelText(/skills.create.description/i), {
      target: { value: 'Does the thing.' },
    });
  }

  /** Find the most-recently-added input row's inner controls. */
  function lastRow() {
    const rows = document.querySelectorAll<HTMLDivElement>(
      '[data-testid^="create-skill-input-row-"]'
    );
    return rows[rows.length - 1];
  }

  it('zero inputs submits a payload without an `inputs` field', async () => {
    hoisted.createWorkflow.mockResolvedValue({ id: 'x', name: 'x', scope: 'user', legacy: false });
    render(<CreateWorkflowForm formId={FORM_ID} onCreated={vi.fn()} />);
    fillRequiredFields();
    fireEvent.submit(document.getElementById(FORM_ID)!);
    await waitFor(() => {
      expect(hoisted.createWorkflow).toHaveBeenCalledWith({
        name: 'My Skill',
        description: 'Does the thing.',
        scope: 'user',
      });
    });
    const payload = hoisted.createWorkflow.mock.calls[0]![0] as Record<string, unknown>;
    expect(payload).not.toHaveProperty('inputs');
  });

  it('one filled input row ships in the payload — additional inputs default to required: false', async () => {
    hoisted.createWorkflow.mockResolvedValue({ id: 'x', name: 'x', scope: 'user', legacy: false });
    render(<CreateWorkflowForm formId={FORM_ID} onCreated={vi.fn()} />);
    fillRequiredFields();

    fireEvent.click(screen.getByTestId('create-skill-add-input'));
    const row = lastRow();
    const [nameInput, descInput] = row.querySelectorAll<HTMLInputElement>('input[type="text"]');
    fireEvent.change(nameInput, { target: { value: 'repo' } });
    fireEvent.change(descInput, { target: { value: 'owner/name' } });

    fireEvent.submit(document.getElementById(FORM_ID)!);
    await waitFor(() => {
      expect(hoisted.createWorkflow).toHaveBeenCalledWith({
        name: 'My Skill',
        description: 'Does the thing.',
        scope: 'user',
        // New default: rows are optional unless the author ticks Required.
        inputs: [{ name: 'repo', required: false, description: 'owner/name' }],
      });
    });
  });

  it('blocks submission when an added input row has no description', async () => {
    hoisted.createWorkflow.mockResolvedValue({ id: 'x', name: 'x', scope: 'user', legacy: false });
    render(<CreateWorkflowForm formId={FORM_ID} onCreated={vi.fn()} />);
    fillRequiredFields();

    fireEvent.click(screen.getByTestId('create-skill-add-input'));
    const row = lastRow();
    const [nameInput] = row.querySelectorAll<HTMLInputElement>('input[type="text"]');
    // Valid name, but description left empty → submission must be blocked.
    fireEvent.change(nameInput, { target: { value: 'repo' } });
    fireEvent.submit(document.getElementById(FORM_ID)!);
    await Promise.resolve();
    expect(hoisted.createWorkflow).not.toHaveBeenCalled();
    expect(screen.getByText(/descriptionError/i)).toBeInTheDocument();

    // Fill the description → now it submits.
    const [, descInput] = row.querySelectorAll<HTMLInputElement>('input[type="text"]');
    fireEvent.change(descInput, { target: { value: 'owner/name' } });
    fireEvent.submit(document.getElementById(FORM_ID)!);
    await waitFor(() => {
      expect(hoisted.createWorkflow).toHaveBeenCalledTimes(1);
    });
  });

  it('ticking Required flips the row to required: true', async () => {
    hoisted.createWorkflow.mockResolvedValue({ id: 'x', name: 'x', scope: 'user', legacy: false });
    render(<CreateWorkflowForm formId={FORM_ID} onCreated={vi.fn()} />);
    fillRequiredFields();

    fireEvent.click(screen.getByTestId('create-skill-add-input'));
    const row = lastRow();
    const [nameInput, descInput] = row.querySelectorAll<HTMLInputElement>('input[type="text"]');
    fireEvent.change(nameInput, { target: { value: 'repo' } });
    fireEvent.change(descInput, { target: { value: 'owner/name' } });
    // Default is unchecked (false); tick it → true.
    const requiredCheckbox = row.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
    fireEvent.click(requiredCheckbox);

    fireEvent.submit(document.getElementById(FORM_ID)!);
    await waitFor(() => {
      expect(hoisted.createWorkflow).toHaveBeenCalledWith(
        expect.objectContaining({
          inputs: [{ name: 'repo', required: true, description: 'owner/name' }],
        })
      );
    });
  });

  it('blocks submission while any row has an invalid name', async () => {
    hoisted.createWorkflow.mockResolvedValue({ id: 'x', name: 'x', scope: 'user', legacy: false });
    render(<CreateWorkflowForm formId={FORM_ID} onCreated={vi.fn()} />);
    fillRequiredFields();

    fireEvent.click(screen.getByTestId('create-skill-add-input'));
    // Add a row but leave the name empty — submission must be blocked.
    fireEvent.submit(document.getElementById(FORM_ID)!);
    await Promise.resolve();
    expect(hoisted.createWorkflow).not.toHaveBeenCalled();

    // Fill the name with an invalid character (leading digit).
    const row = lastRow();
    const [nameInput] = row.querySelectorAll<HTMLInputElement>('input[type="text"]');
    fireEvent.change(nameInput, { target: { value: '2repo' } });
    fireEvent.submit(document.getElementById(FORM_ID)!);
    await Promise.resolve();
    expect(hoisted.createWorkflow).not.toHaveBeenCalled();

    // Inline error visible.
    expect(screen.getByText(/nameError/i)).toBeInTheDocument();
  });

  it('remove row drops it from the payload — submission then succeeds', async () => {
    hoisted.createWorkflow.mockResolvedValue({ id: 'x', name: 'x', scope: 'user', legacy: false });
    render(<CreateWorkflowForm formId={FORM_ID} onCreated={vi.fn()} />);
    fillRequiredFields();

    fireEvent.click(screen.getByTestId('create-skill-add-input'));
    const row = lastRow();
    const removeBtn = row.querySelector<HTMLButtonElement>(
      '[data-testid^="create-skill-remove-input-"]'
    )!;
    fireEvent.click(removeBtn);

    // After removal, zero rows → submission goes through with no
    // `inputs` field at all (back to the no-inputs payload shape).
    fireEvent.submit(document.getElementById(FORM_ID)!);
    await waitFor(() => {
      expect(hoisted.createWorkflow).toHaveBeenCalledTimes(1);
    });
    const payload = hoisted.createWorkflow.mock.calls[0]![0] as Record<string, unknown>;
    expect(payload).not.toHaveProperty('inputs');
  });

  it('integer + required=false (the default) carry through the type + required flags', async () => {
    hoisted.createWorkflow.mockResolvedValue({ id: 'x', name: 'x', scope: 'user', legacy: false });
    render(<CreateWorkflowForm formId={FORM_ID} onCreated={vi.fn()} />);
    fillRequiredFields();

    fireEvent.click(screen.getByTestId('create-skill-add-input'));
    const row = lastRow();
    const [nameInput, descInput] = row.querySelectorAll<HTMLInputElement>('input[type="text"]');
    fireEvent.change(nameInput, { target: { value: 'issue' } });
    // Description is now mandatory per row.
    fireEvent.change(descInput, { target: { value: 'Issue number to work on' } });
    // Flip type → integer; leave Required unticked (false is the default now).
    const typeSelect = row.querySelector<HTMLSelectElement>('select')!;
    fireEvent.change(typeSelect, { target: { value: 'integer' } });

    fireEvent.submit(document.getElementById(FORM_ID)!);
    await waitFor(() => {
      expect(hoisted.createWorkflow).toHaveBeenCalledWith(
        expect.objectContaining({
          inputs: [
            {
              name: 'issue',
              required: false,
              type: 'integer',
              description: 'Issue number to work on',
            },
          ],
        })
      );
    });
  });
});

describe('CreateWorkflowForm — edit mode', () => {
  beforeEach(() => {
    hoisted.createWorkflow.mockReset();
    hoisted.updateWorkflow.mockReset();
    hoisted.describeWorkflow.mockReset();
  });

  const editingSummary = {
    id: 'wf-edit',
    name: 'wf-edit',
    description: 'old desc',
    version: '',
    author: 'me',
    tags: ['t1'],
    platforms: [],
    relatedSkills: [],
    sourceFormat: 'openhuman',
    tools: ['shell'],
    prompts: [],
    location: null,
    resources: [],
    scope: 'user' as const,
    legacy: false,
    warnings: [],
  };

  it('prefills from the summary + describe, then submits via updateWorkflow', async () => {
    hoisted.describeWorkflow.mockResolvedValue({
      id: 'wf-edit',
      name: 'wf-edit',
      when_to_use: 'edit trigger',
      inputs: [{ name: 'repo', type: 'string', required: true, description: 'r' }],
    });
    hoisted.updateWorkflow.mockResolvedValue({
      id: 'wf-edit',
      name: 'wf-edit',
      scope: 'user',
      legacy: false,
    });

    const onCreated = vi.fn();
    render(<CreateWorkflowForm formId={FORM_ID} editing={editingSummary} onCreated={onCreated} />);

    // Prefill: describe is fetched and name is populated from the summary.
    await waitFor(() => expect(hoisted.describeWorkflow).toHaveBeenCalledWith('wf-edit'));
    const nameInput = screen.getByLabelText(/skills.create.name/i) as HTMLInputElement;
    await waitFor(() => expect(nameInput.value).toBe('wf-edit'));

    fireEvent.submit(document.getElementById(FORM_ID)!);
    await waitFor(() =>
      expect(hoisted.updateWorkflow).toHaveBeenCalledWith(
        expect.objectContaining({
          name: 'wf-edit',
          whenToUse: 'edit trigger',
          tags: ['t1'],
          author: 'me',
          allowedTools: ['shell'],
        })
      )
    );
    expect(hoisted.createWorkflow).not.toHaveBeenCalled();
    await waitFor(() => expect(onCreated).toHaveBeenCalled());
  });
});
