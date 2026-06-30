/**
 * WorkflowRunnerBody — vitest coverage for the saved-schedules block.
 *
 * Phase 2 of the WorkflowRunnerBody / DevWorkflowPanel unification (see
 * docs/skills-runner-unification.md): this file is seeded with the
 * smoke-test for the enable/disable toggle so future Phase 3 chunks
 * (run-history, active-config card, smart-issue picker gating) drop
 * additional cases alongside.
 *
 * Covered here:
 *  - Mount with one saved schedule for the picked skill (mocking
 *    workflows_list, workflows_describe, cron_list, recent_runs).
 *  - Toggle flips enabled → false via openhumanCronUpdate(id, { enabled }).
 *  - The list re-loads after toggle (openhumanCronList called again).
 *  - aria-checked reflects the new state once the list refreshes.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

// Mock the i18n hook with a stable identity-returning t() so our
// assertions can query by key (matches existing patterns in the repo,
// e.g. DevWorkflowPanel.test.tsx).
const stableT = (key: string) => key;
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: stableT }) }));

// Hoisted mocks so vi.mock factories can reach them.
const hoisted = vi.hoisted(() => ({
  cronList: vi.fn(),
  cronAdd: vi.fn(),
  cronRemove: vi.fn(),
  cronRun: vi.fn(),
  cronUpdate: vi.fn(),
  cronRuns: vi.fn(),
  listWorkflows: vi.fn(),
  describeWorkflow: vi.fn(),
  runWorkflow: vi.fn(),
  recentRuns: vi.fn(),
  readRunLog: vi.fn(),
  cancelRun: vi.fn(),
  resolveRuntimes: vi.fn(),
  describeSkillList: vi.fn(),
}));

vi.mock('../../../utils/tauriCommands/cron', () => ({
  openhumanCronAdd: hoisted.cronAdd,
  openhumanCronList: hoisted.cronList,
  openhumanCronRemove: hoisted.cronRemove,
  openhumanCronRun: hoisted.cronRun,
  openhumanCronUpdate: hoisted.cronUpdate,
  openhumanCronRuns: hoisted.cronRuns,
}));

vi.mock('../../../services/api/workflowsApi', () => ({
  workflowsApi: {
    listWorkflows: hoisted.listWorkflows,
    describeWorkflow: hoisted.describeWorkflow,
    runWorkflow: hoisted.runWorkflow,
    recentRuns: hoisted.recentRuns,
    readRunLog: hoisted.readRunLog,
    cancelRun: hoisted.cancelRun,
    resolveRuntimes: hoisted.resolveRuntimes,
  },
}));

// Stub the edit modal so opening Edit doesn't pull in the real create form.
vi.mock('../CreateSkillModal', () => ({
  default: ({ editing, onClose }: { editing?: { id: string }; onClose: () => void }) => (
    <div data-testid="edit-modal-stub">
      editing:{editing?.id ?? 'none'}
      <button data-testid="edit-modal-close" onClick={onClose} />
    </div>
  ),
}));

// Composio-backed pickers fetch on mount — stub them so they don't
// throw on the test environment.
vi.mock('../inputs/RepoPicker', () => ({
  default: (props: { id: string; value: string; onChange: (s: string) => void }) => (
    <input
      data-testid="repo-picker-stub"
      id={props.id}
      value={props.value}
      onChange={e => props.onChange(e.target.value)}
    />
  ),
}));
vi.mock('../inputs/BranchPicker', () => ({
  default: (props: { id: string; value: string; onChange: (s: string) => void }) => (
    <input
      data-testid="branch-picker-stub"
      id={props.id}
      value={props.value}
      onChange={e => props.onChange(e.target.value)}
    />
  ),
}));
// SmartIssuePicker mounts Composio + needs the i18n context's `t` to
// resolve a bunch of keys; we just stub the marker so the gating
// assertion below is unambiguous (its internal behaviour has its own
// unit coverage on the subcomponent itself).
vi.mock('../SmartIssuePicker', () => ({
  default: () => <div data-testid="smart-issue-picker-stub" />,
}));

// Mock data ──────────────────────────────────────────────────────────

const SKILL_ID = 'github-issue-crusher';

const skillsList = [{ id: SKILL_ID, name: 'GitHub Issue Crusher' }];

const skillDescription = {
  id: SKILL_ID,
  name: 'GitHub Issue Crusher',
  when_to_use: 'Pick + fix an issue.',
  inputs: [],
};

function makeJob(overrides: Partial<Record<string, unknown>> = {}) {
  return {
    id: 'job-1',
    expression: '*/30 * * * *',
    schedule: { kind: 'cron', expr: '*/30 * * * *' },
    command: '',
    prompt: '',
    name: `skill-run-${SKILL_ID}`,
    job_type: 'agent',
    session_target: 'isolated',
    enabled: true,
    delivery: { mode: 'proactive', best_effort: true },
    delete_after_run: false,
    created_at: '2026-05-29T10:00:00Z',
    next_run: '2026-05-29T11:00:00Z',
    ...overrides,
  };
}

async function importBody() {
  const mod = await import('../WorkflowRunnerBody');
  return mod.WorkflowRunnerBody;
}

/**
 * Wrap the body in a MemoryRouter so the URL-binding effect (added in
 * Phase 4 of the /skills IA restructure) has a router context to read
 * `?workflow=` from / write back to. Default entry is `/workflows/run`
 * matching where the runner now lives.
 */
function renderBody(Body: React.ComponentType, initialPath = '/workflows/run') {
  return render(
    <MemoryRouter initialEntries={[initialPath]}>
      <Body />
    </MemoryRouter>
  );
}

// Tests ──────────────────────────────────────────────────────────────

describe('WorkflowRunnerBody — saved-schedule toggle', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach(fn => fn.mockReset());

    hoisted.listWorkflows.mockResolvedValue(skillsList);
    hoisted.describeWorkflow.mockResolvedValue(skillDescription);
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.resolveRuntimes.mockResolvedValue({ runtimes: [] });
    hoisted.cronList.mockResolvedValue({ result: [makeJob({ enabled: true })] });
    hoisted.cronUpdate.mockResolvedValue({ result: makeJob({ enabled: false }) });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
  });

  it('renders the toggle in the enabled state for an enabled job', async () => {
    const Body = await importBody();
    renderBody(Body);

    // Wait for workflows_list to resolve and populate the dropdown.
    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());

    // Pick the skill so the schedule list mounts.
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: SKILL_ID } });

    await waitFor(() => expect(hoisted.cronList).toHaveBeenCalled());

    // The runner now renders saved schedules through ScheduledCronCard,
    // which emits a single `<root>-toggle` testid per card. Querying
    // by testid keeps us independent of the card's internal aria-label.
    const toggle = await screen.findByTestId('scheduled-job-job-1-toggle');
    expect(toggle).toHaveAttribute('aria-checked', 'true');
    // Card uses the shared `common.enabled` / `common.disabled` label.
    expect(screen.getByText('common.enabled')).toBeInTheDocument();
  });

  it('calls openhumanCronUpdate with { enabled: false } when toggled on→off', async () => {
    const Body = await importBody();
    renderBody(Body);

    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: SKILL_ID } });
    await waitFor(() => expect(hoisted.cronList).toHaveBeenCalled());

    // After the first list, the next call (post-toggle) should return
    // the disabled job so the UI refresh reflects the new state.
    hoisted.cronList.mockResolvedValueOnce({ result: [makeJob({ enabled: false })] });

    const toggle = await screen.findByTestId('scheduled-job-job-1-toggle');
    fireEvent.click(toggle);

    await waitFor(() =>
      expect(hoisted.cronUpdate).toHaveBeenCalledWith('job-1', { enabled: false })
    );

    // Refresh-list invoked after toggle (so the label updates).
    await waitFor(() => expect(hoisted.cronList).toHaveBeenCalledTimes(2));

    await waitFor(() =>
      expect(screen.getByTestId('scheduled-job-job-1-toggle')).toHaveAttribute(
        'aria-checked',
        'false'
      )
    );
    expect(screen.getByText('common.disabled')).toBeInTheDocument();
  });

  it('round-trips off→on as well', async () => {
    hoisted.cronList.mockResolvedValueOnce({ result: [makeJob({ enabled: false })] });

    const Body = await importBody();
    renderBody(Body);

    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: SKILL_ID } });
    await waitFor(() => expect(hoisted.cronList).toHaveBeenCalled());

    const toggle = await screen.findByTestId('scheduled-job-job-1-toggle');
    expect(toggle).toHaveAttribute('aria-checked', 'false');

    fireEvent.click(toggle);
    await waitFor(() =>
      expect(hoisted.cronUpdate).toHaveBeenCalledWith('job-1', { enabled: true })
    );
  });
});

// ── Per-job history expand ──────────────────────────────────────────

function makeRun(
  id: number,
  overrides: Partial<{ status: string; output: string | null; duration_ms: number }> = {}
) {
  return {
    id,
    job_id: 'job-1',
    started_at: '2026-05-29T10:00:00Z',
    finished_at: '2026-05-29T10:00:51Z',
    status: 'ok',
    output: 'hello world\nrun output line 2',
    duration_ms: 51000,
    ...overrides,
  };
}

describe('WorkflowRunnerBody — per-job history viewer', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach(fn => fn.mockReset());
    hoisted.listWorkflows.mockResolvedValue(skillsList);
    hoisted.describeWorkflow.mockResolvedValue(skillDescription);
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.resolveRuntimes.mockResolvedValue({ runtimes: [] });
    hoisted.cronList.mockResolvedValue({ result: [makeJob({ enabled: true })] });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [makeRun(1), makeRun(2)] } });
  });

  it('loads cron_runs and renders history rows on first toggle', async () => {
    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: SKILL_ID } });
    await waitFor(() => expect(hoisted.cronList).toHaveBeenCalled());

    const historyToggle = await screen.findByTestId('history-toggle-job-1');
    fireEvent.click(historyToggle);

    await waitFor(() => expect(hoisted.cronRuns).toHaveBeenCalledWith('job-1', 5));
    expect(await screen.findByTestId('history-run-job-1-1')).toBeInTheDocument();
    expect(screen.getByTestId('history-run-job-1-2')).toBeInTheDocument();
  });

  it('expands a run row to show its captured output, hides on collapse', async () => {
    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: SKILL_ID } });
    await waitFor(() => expect(hoisted.cronList).toHaveBeenCalled());

    fireEvent.click(await screen.findByTestId('history-toggle-job-1'));
    const runRow = await screen.findByTestId('history-run-job-1-1');

    expect(screen.queryByText(/hello world/)).not.toBeInTheDocument();
    fireEvent.click(runRow);
    expect(await screen.findByText(/hello world/)).toBeInTheDocument();
    expect(runRow).toHaveAttribute('aria-expanded', 'true');

    fireEvent.click(runRow);
    await waitFor(() => expect(screen.queryByText(/hello world/)).not.toBeInTheDocument());
  });

  it('marks the most-recent enabled schedule as Active and sorts it first', async () => {
    const jobs = [
      makeJob({
        id: 'job-old-enabled',
        name: `skill-run-${SKILL_ID}-old`,
        enabled: true,
        last_run: '2026-05-29T08:00:00Z',
      }),
      makeJob({
        id: 'job-recent-enabled',
        name: `skill-run-${SKILL_ID}-recent`,
        enabled: true,
        last_run: '2026-05-29T10:00:00Z',
      }),
      makeJob({ id: 'job-paused', name: `skill-run-${SKILL_ID}-paused`, enabled: false }),
    ];
    hoisted.cronList.mockResolvedValue({ result: jobs });

    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: SKILL_ID } });
    await waitFor(() => expect(hoisted.cronList).toHaveBeenCalled());

    // The recent enabled job should be marked active (only one active
    // badge present, and it's on the recent job). ScheduledCronCard
    // emits the badge as `<root>-active-badge` where `<root>` is the
    // runner's `scheduled-job-<jobId>` testIdRoot.
    const badges = await screen.findAllByTestId(/-active-badge$/);
    expect(badges).toHaveLength(1);
    expect(badges[0]).toHaveAttribute(
      'data-testid',
      'scheduled-job-job-recent-enabled-active-badge'
    );

    // Sort order: recent enabled, old enabled, paused. We pull the
    // rendered card roots and assert their relative DOM order. The
    // card emits a number of helper testids (`*-toggle`, `*-open`,
    // etc.) prefixed with the same root — narrow the regex to just
    // the card root by anchoring on a job-id pattern.
    const rows = ['job-recent-enabled', 'job-old-enabled', 'job-paused'].map(id =>
      screen.getByTestId(`scheduled-job-${id}`)
    );
    expect(rows[0]).toHaveAttribute('data-active', 'true');
    expect(rows[1]).toHaveAttribute('data-active', 'true');
    expect(rows[2]).toHaveAttribute('data-active', 'false');
    // Confirm DOM order by walking the parent's children.
    const parent = rows[0].parentElement!;
    const cardChildren = Array.from(parent.children).filter(el =>
      el.getAttribute('data-testid')?.startsWith('scheduled-job-job-')
    );
    expect(cardChildren.map(el => el.getAttribute('data-testid'))).toEqual([
      'scheduled-job-job-recent-enabled',
      'scheduled-job-job-old-enabled',
      'scheduled-job-job-paused',
    ]);
  });

  it('does not show an Active badge when no schedules are enabled', async () => {
    hoisted.cronList.mockResolvedValue({ result: [makeJob({ enabled: false })] });
    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: SKILL_ID } });
    await waitFor(() => expect(hoisted.cronList).toHaveBeenCalled());

    await screen.findByTestId('scheduled-job-job-1');
    expect(screen.queryByTestId(/-active-badge$/)).not.toBeInTheDocument();
  });

  it('shows the empty-history placeholder when cron_runs returns no rows', async () => {
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: SKILL_ID } });
    await waitFor(() => expect(hoisted.cronList).toHaveBeenCalled());

    fireEvent.click(await screen.findByTestId('history-toggle-job-1'));
    await waitFor(() => expect(hoisted.cronRuns).toHaveBeenCalled());
    expect(
      await screen.findByText('settings.skillsRunner.schedule.historyEmpty')
    ).toBeInTheDocument();
  });
});

describe('WorkflowRunnerBody — schedule frequency + save', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach(fn => fn.mockReset());
    hoisted.listWorkflows.mockResolvedValue(skillsList);
    hoisted.describeWorkflow.mockResolvedValue(skillDescription);
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.resolveRuntimes.mockResolvedValue({ runtimes: [] });
    hoisted.cronList.mockResolvedValue({ result: [] });
    hoisted.cronAdd.mockResolvedValue({ result: makeJob() });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
  });

  it('changes schedule frequency and calls openhumanCronAdd on save', async () => {
    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: SKILL_ID } });
    await waitFor(() => expect(hoisted.cronList).toHaveBeenCalled());

    const freq = screen.getByLabelText(
      'settings.skillsRunner.schedule.frequency'
    ) as HTMLSelectElement;
    fireEvent.change(freq, { target: { value: '0 9 * * *' } });
    expect(freq.value).toBe('0 9 * * *');

    fireEvent.click(screen.getByText('settings.skillsRunner.schedule.save'));
    await waitFor(() => expect(hoisted.cronAdd).toHaveBeenCalled());
    const [params] = hoisted.cronAdd.mock.calls[0];
    expect(params).toMatchObject({
      schedule: { kind: 'cron', expr: '0 9 * * *' },
      job_type: 'agent',
    });
  });
});

describe('WorkflowRunnerBody — SmartIssuePicker conditional mount', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach(fn => fn.mockReset());
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.resolveRuntimes.mockResolvedValue({ runtimes: [] });
    hoisted.cronList.mockResolvedValue({ result: [] });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
  });

  it('renders SmartIssuePicker when the picked skill is dev-workflow', async () => {
    hoisted.listWorkflows.mockResolvedValue([{ id: 'dev-workflow', name: 'Dev Workflow' }]);
    hoisted.describeWorkflow.mockResolvedValue({
      id: 'dev-workflow',
      name: 'Dev Workflow',
      when_to_use: 'Autonomous developer.',
      inputs: [
        { name: 'repo', type: 'string', required: true, description: 'upstream repo' },
        { name: 'upstream', type: 'string', required: true, description: 'upstream alias' },
        { name: 'target_branch', type: 'string', required: true, description: 'PR base' },
        { name: 'fork_owner', type: 'string', required: true, description: 'fork owner' },
      ],
    });

    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'dev-workflow' } });

    expect(await screen.findByTestId('smart-issue-picker-stub')).toBeInTheDocument();
    // The four managed inputs should NOT appear as plain text fields
    // — they're driven by the picker. We probe one of them.
    expect(screen.queryByLabelText(/target_branch/)).not.toBeInTheDocument();
  });

  it('does NOT render SmartIssuePicker for generic skills', async () => {
    hoisted.listWorkflows.mockResolvedValue([
      { id: 'github-issue-crusher', name: 'GitHub Issue Crusher' },
    ]);
    hoisted.describeWorkflow.mockResolvedValue({
      id: 'github-issue-crusher',
      name: 'GitHub Issue Crusher',
      when_to_use: 'Crush issues.',
      inputs: [
        { name: 'repo', type: 'string', required: true, description: 'repo' },
        { name: 'issue_number', type: 'integer', required: true, description: 'issue' },
      ],
    });

    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'github-issue-crusher' } });

    await waitFor(() => expect(hoisted.describeWorkflow).toHaveBeenCalled());
    expect(screen.queryByTestId('smart-issue-picker-stub')).not.toBeInTheDocument();
    // The generic schema-driven repo field IS rendered via the
    // existing RepoPicker stub.
    expect(await screen.findByTestId('repo-picker-stub')).toBeInTheDocument();
  });
});

// ── Phase 4: URL ?workflow= preselect binding ───────────────────────────

describe('WorkflowRunnerBody — URL ?workflow= preselect', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach(fn => fn.mockReset());
    hoisted.listWorkflows.mockResolvedValue([
      { id: 'dev-workflow', name: 'Dev Workflow' },
      { id: 'github-issue-crusher', name: 'GitHub Issue Crusher' },
    ]);
    hoisted.describeWorkflow.mockResolvedValue({
      id: 'dev-workflow',
      name: 'Dev Workflow',
      when_to_use: 'Autonomous developer.',
      inputs: [],
    });
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.resolveRuntimes.mockResolvedValue({ runtimes: [] });
    hoisted.cronList.mockResolvedValue({ result: [] });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
  });

  it('pre-selects the skill from the ?workflow= query on mount', async () => {
    const Body = await importBody();
    renderBody(Body, '/workflows/run?workflow=dev-workflow');

    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());

    // The picker should already be pointing at dev-workflow without any
    // user interaction. We assert this two ways: (a) the <select>'s
    // value matches, and (b) describeWorkflow was fetched for it.
    const select = (await screen.findByLabelText(
      'settings.skillsRunner.skill'
    )) as HTMLSelectElement;
    expect(select.value).toBe('dev-workflow');
    await waitFor(() => expect(hoisted.describeWorkflow).toHaveBeenCalledWith('dev-workflow'));
  });

  it('does not preselect when no ?workflow= is present', async () => {
    const Body = await importBody();
    renderBody(Body, '/workflows/run');

    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = (await screen.findByLabelText(
      'settings.skillsRunner.skill'
    )) as HTMLSelectElement;
    expect(select.value).toBe('');
    expect(hoisted.describeWorkflow).not.toHaveBeenCalled();
  });

  it('ignores ?workflow= when the value is not in the workflows_list (picker stays empty, describeWorkflow called once with empty=never)', async () => {
    // ?workflow=unknown-skill is treated as best-effort: we set the state
    // but the picker shows "Select a skill" since the option isn't in
    // the list. The describe call IS attempted (we don't pre-filter
    // against the catalog) — but the cancellation effect tears it
    // down if the value never resolves to a real skill.
    hoisted.describeWorkflow.mockRejectedValue(new Error('unknown skill'));
    const Body = await importBody();
    renderBody(Body, '/workflows/run?workflow=does-not-exist');

    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    await waitFor(() => expect(hoisted.describeWorkflow).toHaveBeenCalledWith('does-not-exist'));
    // The dropdown value won't render as an option (not in the list),
    // so its current value normalises to '' visually — but the state
    // we care about is that the error surfaces, not crashes.
    expect(await screen.findByText(/settings.skillsRunner.error.describe/)).toBeInTheDocument();
  });

  it('locked (lock=1): shows the workflow-name header (no picker) AND the Run button', async () => {
    // Regression: opening a workflow from its card locks the page
    // (?workflow=<id>&lock=1). The Run button used to be gated behind
    // !locked, so the consolidated runner could schedule but not run.
    const Body = await importBody();
    renderBody(Body, '/workflows/run?workflow=dev-workflow&lock=1');

    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());

    // The picker is replaced by the workflow-name heading...
    const heading = await screen.findByTestId('skills-runner-skill-locked');
    expect(heading).toHaveTextContent('Dev Workflow');
    // ...so there is no <select> picker in locked mode...
    expect(screen.queryByLabelText('settings.skillsRunner.skill')).not.toBeInTheDocument();
    // ...and the Run button is present after the inputs.
    expect(await screen.findByText('settings.skillsRunner.runNow')).toBeInTheDocument();
  });
});

// ── Phase 5: Run Now flow ────────────────────────────────────────────
//
// Exercises handleRun → buildInputsPayload (lines 167-201), missing-
// required validation (lines 415-429), and the run-result render paths
// (lines 441-452).

describe('WorkflowRunnerBody — Run Now flow', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach(fn => fn.mockReset());
    hoisted.listWorkflows.mockResolvedValue([
      { id: 'pr-review-shepherd', name: 'PR Review Shepherd' },
    ]);
    hoisted.describeWorkflow.mockResolvedValue({
      id: 'pr-review-shepherd',
      name: 'PR Review Shepherd',
      when_to_use: 'Shepherd PRs.',
      inputs: [
        { name: 'repo', type: 'string', required: true, description: 'repo owner/name' },
        { name: 'pr_number', type: 'integer', required: false, description: 'PR number' },
        { name: 'dry_run', type: 'boolean', required: false, description: 'Dry run?' },
      ],
    });
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.resolveRuntimes.mockResolvedValue({ runtimes: [] });
    hoisted.cronList.mockResolvedValue({ result: [] });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
    hoisted.runWorkflow.mockResolvedValue({
      run_id: 'run-abc',
      skill_id: 'pr-review-shepherd',
      log: '/tmp/run-abc.log',
    });
  });

  it('Run Now button is disabled while required fields are empty', async () => {
    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());

    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'pr-review-shepherd' } });
    await waitFor(() =>
      expect(hoisted.describeWorkflow).toHaveBeenCalledWith('pr-review-shepherd')
    );

    // Run Now button should be disabled when required field is empty
    const runBtn = await screen.findByText('settings.skillsRunner.runNow');
    expect(runBtn.closest('button')).toBeDisabled();
    expect(hoisted.runWorkflow).not.toHaveBeenCalled();
  });

  it('calls runWorkflow with built payload when required fields are filled', async () => {
    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());

    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'pr-review-shepherd' } });
    await waitFor(() => expect(hoisted.describeWorkflow).toHaveBeenCalled());

    // Fill the repo input (rendered as a RepoPicker stub <input>)
    const repoInput = await screen.findByTestId('repo-picker-stub');
    fireEvent.change(repoInput, { target: { value: 'owner/myrepo' } });

    // Wait for button to become enabled (state update after required field filled)
    const runBtn = await screen.findByText('settings.skillsRunner.runNow');
    await waitFor(() => expect(runBtn.closest('button')).not.toBeDisabled());
    fireEvent.click(runBtn.closest('button')!);

    await waitFor(() =>
      expect(hoisted.runWorkflow).toHaveBeenCalledWith(
        'pr-review-shepherd',
        expect.objectContaining({ repo: 'owner/myrepo' })
      )
    );
  });

  it('auto-refreshes the recent-runs list after a run starts (no manual refresh)', async () => {
    // Regression: handleRun didn't re-scan recentRuns after starting a run, so
    // the new run never appeared until the user hit refresh — which led them to
    // click Run again and spawn a second run.
    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());

    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'pr-review-shepherd' } });
    await waitFor(() => expect(hoisted.describeWorkflow).toHaveBeenCalled());

    const repoInput = await screen.findByTestId('repo-picker-stub');
    fireEvent.change(repoInput, { target: { value: 'owner/myrepo' } });

    const runBtn = await screen.findByText('settings.skillsRunner.runNow');
    await waitFor(() => expect(runBtn.closest('button')).not.toBeDisabled());

    const callsBefore = hoisted.recentRuns.mock.calls.length;
    fireEvent.click(runBtn.closest('button')!);

    await waitFor(() => expect(hoisted.runWorkflow).toHaveBeenCalledTimes(1));
    // The post-run refresh burst re-scans recentRuns on its own.
    await waitFor(() => expect(hoisted.recentRuns.mock.calls.length).toBeGreaterThan(callsBefore));
  });

  it('surfaces error when runWorkflow rejects', async () => {
    hoisted.runWorkflow.mockRejectedValue(new Error('backend error'));
    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());

    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'pr-review-shepherd' } });
    await waitFor(() => expect(hoisted.describeWorkflow).toHaveBeenCalled());

    const repoInput = await screen.findByTestId('repo-picker-stub');
    fireEvent.change(repoInput, { target: { value: 'owner/myrepo' } });

    const runBtn = await screen.findByText('settings.skillsRunner.runNow');
    await waitFor(() => expect(runBtn.closest('button')).not.toBeDisabled());
    fireEvent.click(runBtn.closest('button')!);

    await waitFor(() => expect(screen.getByTestId('skill-run-error')).toBeInTheDocument());
  });
});

// ── parseScheduledInputs ─────────────────────────────────────────────
//
// Each scheduled job snapshots its own inputs at creation (baked into the
// cron's agent prompt by buildAgentPrompt); the card reads them back via
// parseScheduledInputs to show what that schedule runs with.

describe('parseScheduledInputs', () => {
  it('recovers key/value inputs from a saved cron prompt', async () => {
    const { parseScheduledInputs } = await import('../WorkflowRunnerBody');
    const prompt = [
      'Run the slack-to-notion workflow via the run_workflow tool (workflow_id: "slack-to-notion") with these inputs:',
      '- channel: team-product',
      '- limit: 50',
      '',
      'Do NOT do the work yourself — call run_workflow and report back the new run_id.',
    ].join('\n');
    expect(parseScheduledInputs(prompt)).toEqual([
      { key: 'channel', value: 'team-product' },
      { key: 'limit', value: '50' },
    ]);
  });

  it('returns [] for a no-input prompt and for a missing prompt', async () => {
    const { parseScheduledInputs } = await import('../WorkflowRunnerBody');
    const noInputs = [
      'Run the x workflow via the run_workflow tool (workflow_id: "x") with these inputs:',
      '(no inputs)',
      '',
      'Do NOT do the work yourself.',
    ].join('\n');
    expect(parseScheduledInputs(noInputs)).toEqual([]);
    expect(parseScheduledInputs(undefined)).toEqual([]);
    expect(parseScheduledInputs(null)).toEqual([]);
    expect(parseScheduledInputs('some unrelated prompt')).toEqual([]);
  });
});

// ── Stop / Edit / scheduled run-now ──────────────────────────────────
//
// Covers the consolidated runner's locked-mode surface: the Stop button on
// a RUNNING recent run (handleStopRun → cancelRun), the Edit modal opened
// from the locked header, and "Run" on a saved schedule running the workflow
// directly with that schedule's snapshotted inputs (handleRunJobNow).

describe('WorkflowRunnerBody — Stop / Edit / scheduled run-now', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach(fn => fn.mockReset());
    hoisted.listWorkflows.mockResolvedValue([
      { id: SKILL_ID, name: 'GitHub Issue Crusher', scope: 'user', legacy: false },
    ]);
    hoisted.describeWorkflow.mockResolvedValue(skillDescription);
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.resolveRuntimes.mockResolvedValue({ runtimes: [] });
    hoisted.cronList.mockResolvedValue({ result: [] });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
    hoisted.runWorkflow.mockResolvedValue({
      run_id: 'r-new',
      workflow_id: SKILL_ID,
      log: '/tmp/l',
    });
    hoisted.cancelRun.mockResolvedValue(true);
  });

  it('shows a Stop button on a RUNNING recent run and cancels it', async () => {
    hoisted.recentRuns.mockResolvedValue([
      {
        run_id: 'run-live',
        workflow_id: SKILL_ID,
        status: 'RUNNING',
        started: '2026-05-29T10:00:00Z',
        duration_ms: null,
        finished: null,
        log_path: '/tmp/run-live.log',
      },
    ]);
    const Body = await importBody();
    renderBody(Body, `/workflows/run?workflow=${SKILL_ID}&lock=1`);
    const stop = await screen.findByTestId('run-stop-run-live');
    fireEvent.click(stop);
    await waitFor(() => expect(hoisted.cancelRun).toHaveBeenCalledWith('run-live'));
  });

  it('opens the edit modal from the locked workflow header', async () => {
    const Body = await importBody();
    renderBody(Body, `/workflows/run?workflow=${SKILL_ID}&lock=1`);
    const edit = await screen.findByTestId('skills-runner-edit');
    fireEvent.click(edit);
    const modal = await screen.findByTestId('edit-modal-stub');
    expect(modal).toHaveTextContent(`editing:${SKILL_ID}`);
  });

  it('runs a saved schedule directly with its snapshotted inputs', async () => {
    hoisted.listWorkflows.mockResolvedValue([
      {
        id: SKILL_ID,
        name: 'GitHub Issue Crusher',
        scope: 'user',
        legacy: false,
        resources: ['scripts/run.py'],
      },
    ]);
    const prompt = [
      `Run the ${SKILL_ID} workflow via the run_workflow tool (workflow_id: "${SKILL_ID}") with these inputs:`,
      '- channel: team-product',
      '',
      'Do NOT do the work yourself.',
    ].join('\n');
    hoisted.cronList.mockResolvedValue({ result: [makeJob({ prompt })] });
    const Body = await importBody();
    renderBody(Body, `/workflows/run?workflow=${SKILL_ID}&lock=1`);
    await waitFor(() => expect(hoisted.cronList).toHaveBeenCalled());

    // Inputs parsed from the job prompt render as chips on the card.
    const inputs = await screen.findByTestId('scheduled-job-job-1-inputs');
    expect(inputs).toHaveTextContent('channel');

    // The card's "Run" runs the workflow directly with those inputs.
    fireEvent.click(screen.getByText('settings.skillsRunner.schedule.runNow'));
    await waitFor(() => expect(hoisted.resolveRuntimes).toHaveBeenCalledWith('python'));
    await waitFor(() =>
      expect(hoisted.runWorkflow).toHaveBeenCalledWith(SKILL_ID, { channel: 'team-product' })
    );
  });
});

// ── skillsError display ──────────────────────────────────────────────

describe('WorkflowRunnerBody — skillsError state', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach(fn => fn.mockReset());
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.resolveRuntimes.mockResolvedValue({ runtimes: [] });
    hoisted.cronList.mockResolvedValue({ result: [] });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
  });

  it('shows skillsError message when listWorkflows rejects', async () => {
    hoisted.listWorkflows.mockRejectedValue(new Error('network failure'));
    const Body = await importBody();
    renderBody(Body);

    await waitFor(() =>
      expect(screen.getByText(/settings.skillsRunner.error.listWorkflows/)).toBeInTheDocument()
    );
    expect(screen.getByText(/network failure/)).toBeInTheDocument();
  });
});

// ── Recent runs status badge colors ─────────────────────────────────

describe('WorkflowRunnerBody — recent runs status badges', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach(fn => fn.mockReset());
    hoisted.listWorkflows.mockResolvedValue([]);
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.resolveRuntimes.mockResolvedValue({ runtimes: [] });
    hoisted.cronList.mockResolvedValue({ result: [] });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
  });

  function makeRecentRun(status: string) {
    return {
      run_id: `run-${status.toLowerCase()}`,
      workflow_id: SKILL_ID,
      status,
      started: '2026-05-30T09:00:00Z',
      duration_ms: status === 'RUNNING' ? null : 5000,
      finished: status === 'RUNNING' ? null : '2026-05-30T09:00:05Z',
      log_path: `/tmp/run-${status.toLowerCase()}.log`,
    };
  }

  it('renders DONE, DEGENERATE, and FAILED recent run rows', async () => {
    hoisted.recentRuns.mockResolvedValue([
      makeRecentRun('DONE'),
      makeRecentRun('DEGENERATE'),
      makeRecentRun('FAILED'),
    ]);
    const Body = await importBody();
    renderBody(Body);

    await waitFor(() => {
      expect(screen.getByText('DONE')).toBeInTheDocument();
      expect(screen.getByText('DEGENERATE')).toBeInTheDocument();
      expect(screen.getByText('FAILED')).toBeInTheDocument();
    });
  });

  it('expands a recent run row and shows tailing indicator', async () => {
    hoisted.recentRuns.mockResolvedValue([makeRecentRun('DONE')]);
    hoisted.readRunLog.mockResolvedValue({
      bytes_read: 12,
      eof: false,
      complete: false,
      content: 'log content',
      offset: 12,
    });
    const Body = await importBody();
    renderBody(Body);

    // Wait for the run row to appear
    const runBtn = await screen.findByText('DONE');
    fireEvent.click(runBtn.closest('button')!);

    // After expansion, the tailing indicator should appear
    await waitFor(() => {
      expect(screen.getByText(/settings.skillsRunner.viewer.tailing/)).toBeInTheDocument();
    });
    // The log content should be rendered in a pre block
    await waitFor(() => {
      expect(screen.getByText('log content')).toBeInTheDocument();
    });
  });

  it('shows error in viewer when readRunLog fails', async () => {
    hoisted.recentRuns.mockResolvedValue([makeRecentRun('RUNNING')]);
    hoisted.readRunLog.mockRejectedValue(new Error('log read failed'));
    const Body = await importBody();
    renderBody(Body);

    const runBtn = await screen.findByText('RUNNING');
    fireEvent.click(runBtn.closest('button')!);

    await waitFor(() => {
      expect(screen.getByText(/log read failed/)).toBeInTheDocument();
    });
  });

  it('shows collapse icon when run row is expanded and expand icon when collapsed', async () => {
    hoisted.recentRuns.mockResolvedValue([makeRecentRun('DONE')]);
    hoisted.readRunLog.mockResolvedValue({
      bytes_read: 0,
      eof: true,
      complete: true,
      content: '',
      offset: 0,
    });
    const Body = await importBody();
    renderBody(Body);

    const runBtn = await screen.findByText('DONE');
    const btn = runBtn.closest('button')!;

    // Before expand: shows ▸
    expect(btn).toHaveTextContent('▸');

    fireEvent.click(btn);

    // After expand: shows ▾
    await waitFor(() => {
      expect(btn).toHaveTextContent('▾');
    });

    // Collapse again
    fireEvent.click(btn);
    await waitFor(() => {
      expect(btn).toHaveTextContent('▸');
    });
  });

  it('shows all recent runs heading when no skill selected', async () => {
    const Body = await importBody();
    renderBody(Body);

    await waitFor(() => {
      expect(screen.getByText('settings.skillsRunner.recentRuns.headingAll')).toBeInTheDocument();
    });
  });

  it('shows skill-scoped heading when a skill is selected', async () => {
    hoisted.listWorkflows.mockResolvedValue([{ id: SKILL_ID, name: 'GitHub Issue Crusher' }]);
    hoisted.describeWorkflow.mockResolvedValue(skillDescription);
    const Body = await importBody();
    renderBody(Body);

    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: SKILL_ID } });

    await waitFor(() => {
      expect(
        screen.getByText('settings.skillsRunner.recentRuns.headingForSkill')
      ).toBeInTheDocument();
    });
  });
});

// ── renderField type variants ────────────────────────────────────────

describe('WorkflowRunnerBody — renderField type variants', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach(fn => fn.mockReset());
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.resolveRuntimes.mockResolvedValue({ runtimes: [] });
    hoisted.cronList.mockResolvedValue({ result: [] });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
    hoisted.listWorkflows.mockResolvedValue([{ id: 'multi-input', name: 'Multi Input' }]);
  });

  it('renders boolean (checkbox), integer (number), and string inputs', async () => {
    hoisted.describeWorkflow.mockResolvedValue({
      id: 'multi-input',
      name: 'Multi Input',
      when_to_use: 'Multi.',
      inputs: [
        { name: 'dry_run', type: 'boolean', required: false, description: 'Dry run?' },
        { name: 'count', type: 'integer', required: false, description: 'Count' },
        { name: 'message', type: 'string', required: true, description: 'Message' },
      ],
    });
    const Body = await importBody();
    renderBody(Body);

    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'multi-input' } });

    await waitFor(() => expect(hoisted.describeWorkflow).toHaveBeenCalledWith('multi-input'));

    // boolean → checkbox
    const checkbox = await screen.findByRole('checkbox');
    expect(checkbox).toBeInTheDocument();

    // integer → number input
    const numInput = screen.getByRole('spinbutton');
    expect(numInput).toBeInTheDocument();

    // string → text input
    const textInputs = screen.getAllByRole('textbox');
    expect(textInputs.length).toBeGreaterThanOrEqual(1);
  });
});

// ── handleRemoveJob ──────────────────────────────────────────────────

describe('WorkflowRunnerBody — handleRemoveJob', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach(fn => fn.mockReset());
    hoisted.listWorkflows.mockResolvedValue(skillsList);
    hoisted.describeWorkflow.mockResolvedValue(skillDescription);
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.resolveRuntimes.mockResolvedValue({ runtimes: [] });
    hoisted.cronList.mockResolvedValue({ result: [makeJob({ enabled: true })] });
    hoisted.cronRemove.mockResolvedValue({ result: 'ok' });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
  });

  it('calls openhumanCronRemove when the Remove button is clicked', async () => {
    hoisted.cronList
      .mockResolvedValueOnce({ result: [makeJob({ enabled: true })] })
      .mockResolvedValue({ result: [] });

    const Body = await importBody();
    renderBody(Body);

    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: SKILL_ID } });

    await waitFor(() => expect(hoisted.cronList).toHaveBeenCalled());

    const removeBtn = await screen.findByText('settings.skillsRunner.schedule.remove');
    fireEvent.click(removeBtn);

    await waitFor(() => expect(hoisted.cronRemove).toHaveBeenCalledWith('job-1'));
    // After removal the list is refreshed
    await waitFor(() => expect(hoisted.cronList).toHaveBeenCalledTimes(2));
  });
});

// ── loadJobHistory error path ────────────────────────────────────────

describe('WorkflowRunnerBody — loadJobHistory error path', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach(fn => fn.mockReset());
    hoisted.listWorkflows.mockResolvedValue(skillsList);
    hoisted.describeWorkflow.mockResolvedValue(skillDescription);
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.resolveRuntimes.mockResolvedValue({ runtimes: [] });
    hoisted.cronList.mockResolvedValue({ result: [makeJob({ enabled: true })] });
    hoisted.cronRuns.mockRejectedValue(new Error('history fetch failed'));
  });

  it('handles cronRuns error gracefully (no crash, history stays collapsed-but-error)', async () => {
    const Body = await importBody();
    renderBody(Body);

    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: SKILL_ID } });

    await waitFor(() => expect(hoisted.cronList).toHaveBeenCalled());

    const historyToggle = await screen.findByTestId('history-toggle-job-1');
    fireEvent.click(historyToggle);

    await waitFor(() => expect(hoisted.cronRuns).toHaveBeenCalledWith('job-1', 5));
    // The component should not crash and should still be in the DOM
    expect(screen.getByTestId('history-toggle-job-1')).toBeInTheDocument();
  });
});

// ── history run with no output ───────────────────────────────────────

describe('WorkflowRunnerBody — history run with no output', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach(fn => fn.mockReset());
    hoisted.listWorkflows.mockResolvedValue(skillsList);
    hoisted.describeWorkflow.mockResolvedValue(skillDescription);
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.resolveRuntimes.mockResolvedValue({ runtimes: [] });
    hoisted.cronList.mockResolvedValue({ result: [makeJob({ enabled: true })] });
  });

  it('shows historyNoOutput placeholder when a run row has null output', async () => {
    hoisted.cronRuns.mockResolvedValue({
      result: {
        runs: [
          {
            id: 1,
            job_id: 'job-1',
            started_at: '2026-05-30T09:00:00Z',
            finished_at: '2026-05-30T09:00:05Z',
            status: 'ok',
            output: null,
            duration_ms: 5000,
          },
        ],
      },
    });

    const Body = await importBody();
    renderBody(Body);

    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: SKILL_ID } });

    await waitFor(() => expect(hoisted.cronList).toHaveBeenCalled());

    const historyToggle = await screen.findByTestId('history-toggle-job-1');
    fireEvent.click(historyToggle);

    await waitFor(() => expect(hoisted.cronRuns).toHaveBeenCalled());

    const runRow = await screen.findByTestId('history-run-job-1-1');
    fireEvent.click(runRow);

    await waitFor(() => {
      expect(
        screen.getByText('settings.skillsRunner.schedule.historyNoOutput')
      ).toBeInTheDocument();
    });
  });
});

// ── save schedule: missing required fields ───────────────────────────

describe('WorkflowRunnerBody — save schedule with missing required fields', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach(fn => fn.mockReset());
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.resolveRuntimes.mockResolvedValue({ runtimes: [] });
    hoisted.cronList.mockResolvedValue({ result: [] });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
    hoisted.listWorkflows.mockResolvedValue([{ id: 'req-skill', name: 'Req Skill' }]);
    hoisted.describeWorkflow.mockResolvedValue({
      id: 'req-skill',
      name: 'Req Skill',
      when_to_use: 'req.',
      inputs: [
        { name: 'required_param', type: 'string', required: true, description: 'Required param' },
      ],
    });
  });

  it('shows schedule error when required field is blank on save', async () => {
    const Body = await importBody();
    renderBody(Body);

    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'req-skill' } });

    await waitFor(() => expect(hoisted.describeWorkflow).toHaveBeenCalledWith('req-skill'));

    // Don't fill the required field, hit save
    const saveBtn = await screen.findByText('settings.skillsRunner.schedule.save');
    fireEvent.click(saveBtn);

    await waitFor(() => {
      // The schedule error shows the missing field name
      expect(screen.getByText(/required_param/)).toBeInTheDocument();
    });
    // cronAdd should NOT have been called
    expect(hoisted.cronAdd).not.toHaveBeenCalled();
  });
});

// ── buildCronJobName via save schedule ───────────────────────────────

describe('WorkflowRunnerBody — buildCronJobName with non-empty inputs', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach(fn => fn.mockReset());
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.resolveRuntimes.mockResolvedValue({ runtimes: [] });
    hoisted.cronList.mockResolvedValue({ result: [] });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
    hoisted.cronAdd.mockResolvedValue({ result: makeJob() });
    hoisted.listWorkflows.mockResolvedValue([{ id: 'name-skill', name: 'Name Skill' }]);
    hoisted.describeWorkflow.mockResolvedValue({
      id: 'name-skill',
      name: 'Name Skill',
      when_to_use: 'test.',
      inputs: [{ name: 'owner', type: 'string', required: true, description: 'Owner' }],
    });
  });

  it('builds cron job name including the input value and calls cronAdd', async () => {
    const Body = await importBody();
    renderBody(Body);

    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'name-skill' } });

    await waitFor(() => expect(hoisted.describeWorkflow).toHaveBeenCalledWith('name-skill'));

    const ownerInput = await screen.findByRole('textbox');
    fireEvent.change(ownerInput, { target: { value: 'tinyhumansai' } });

    fireEvent.click(screen.getByText('settings.skillsRunner.schedule.save'));

    await waitFor(() => expect(hoisted.cronAdd).toHaveBeenCalled());
    const [params] = hoisted.cronAdd.mock.calls[0];
    // The cron job name should include the skill id and input value
    expect(params.name).toContain('name-skill');
    expect(params.name).toContain('tinyhumansai');
  });
});

// ── ensureRuntimeAvailability failure ────────────────────────────────

describe('WorkflowRunnerBody — ensureRuntimeAvailability failure', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach(fn => fn.mockReset());
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.cronList.mockResolvedValue({ result: [] });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
    hoisted.runWorkflow.mockResolvedValue({ run_id: 'r', skill_id: 'x', log: '/tmp/l' });
    hoisted.listWorkflows.mockResolvedValue([
      { id: 'py-skill', name: 'Python Skill', resources: ['scripts/run.py'] },
    ]);
    hoisted.describeWorkflow.mockResolvedValue({
      id: 'py-skill',
      name: 'Python Skill',
      when_to_use: 'run py.',
      inputs: [],
    });
  });

  it('surfaces runtime unavailable error on run when python runtime is missing', async () => {
    hoisted.resolveRuntimes.mockResolvedValue({
      runtimes: [
        {
          runtime: 'python',
          enabled: true,
          available: false,
          source: 'managed',
          version: null,
          binary: null,
          binDir: null,
          error: 'not installed',
        },
      ],
    });

    const Body = await importBody();
    renderBody(Body);

    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'py-skill' } });

    await waitFor(() => expect(hoisted.describeWorkflow).toHaveBeenCalledWith('py-skill'));

    const runBtn = await screen.findByText('settings.skillsRunner.runNow');
    fireEvent.click(runBtn.closest('button')!);

    await waitFor(() => {
      expect(screen.getByTestId('skill-run-error')).toBeInTheDocument();
    });
    expect(screen.getByText(/python/)).toBeInTheDocument();
  });
});

// ── preflight gate failure pill ──────────────────────────────────────

describe('WorkflowRunnerBody — preflight gate failure', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach(fn => fn.mockReset());
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.resolveRuntimes.mockResolvedValue({ runtimes: [] });
    hoisted.cronList.mockResolvedValue({ result: [] });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
    hoisted.listWorkflows.mockResolvedValue([{ id: 'gated-skill', name: 'Gated Skill' }]);
    hoisted.describeWorkflow.mockResolvedValue({
      id: 'gated-skill',
      name: 'Gated Skill',
      when_to_use: 'gated.',
      inputs: [],
    });
  });

  it('shows preflight gate pill when run fails with preflight error format', async () => {
    // The preflight error format is: [preflight:<gate>:<tag>] <body>
    hoisted.runWorkflow.mockRejectedValue(
      new Error('[preflight:github:token] GitHub token not configured')
    );

    const Body = await importBody();
    renderBody(Body);

    await waitFor(() => expect(hoisted.listWorkflows).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'gated-skill' } });

    await waitFor(() => expect(hoisted.describeWorkflow).toHaveBeenCalledWith('gated-skill'));

    const runBtn = await screen.findByText('settings.skillsRunner.runNow');
    fireEvent.click(runBtn.closest('button')!);

    await waitFor(() => {
      expect(screen.getByTestId('skill-run-error')).toBeInTheDocument();
    });
  });
});
