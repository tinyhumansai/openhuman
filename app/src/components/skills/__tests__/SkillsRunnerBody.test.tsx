/**
 * SkillsRunnerBody — vitest coverage for the saved-schedules block.
 *
 * Phase 2 of the SkillsRunnerBody / DevWorkflowPanel unification (see
 * docs/skills-runner-unification.md): this file is seeded with the
 * smoke-test for the enable/disable toggle so future Phase 3 chunks
 * (run-history, active-config card, smart-issue picker gating) drop
 * additional cases alongside.
 *
 * Covered here:
 *  - Mount with one saved schedule for the picked skill (mocking
 *    skills_list, skills_describe, cron_list, recent_runs).
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
  listSkills: vi.fn(),
  describeSkill: vi.fn(),
  runSkill: vi.fn(),
  recentRuns: vi.fn(),
  readRunLog: vi.fn(),
}));

vi.mock('../../../utils/tauriCommands/cron', () => ({
  openhumanCronAdd: hoisted.cronAdd,
  openhumanCronList: hoisted.cronList,
  openhumanCronRemove: hoisted.cronRemove,
  openhumanCronRun: hoisted.cronRun,
  openhumanCronUpdate: hoisted.cronUpdate,
  openhumanCronRuns: hoisted.cronRuns,
}));

vi.mock('../../../services/api/skillsApi', () => ({
  skillsApi: {
    listSkills: hoisted.listSkills,
    describeSkill: hoisted.describeSkill,
    runSkill: hoisted.runSkill,
    recentRuns: hoisted.recentRuns,
    readRunLog: hoisted.readRunLog,
  },
}));

// Composio-backed pickers fetch on mount — stub them so they don't
// throw on the test environment.
vi.mock('../inputs/RepoPicker', () => ({
  default: (props: { id: string; value: string; onChange: (s: string) => void }) => (
    <input
      data-testid="repo-picker-stub"
      id={props.id}
      value={props.value}
      onChange={(e) => props.onChange(e.target.value)}
    />
  ),
}));
vi.mock('../inputs/BranchPicker', () => ({
  default: (props: { id: string; value: string; onChange: (s: string) => void }) => (
    <input
      data-testid="branch-picker-stub"
      id={props.id}
      value={props.value}
      onChange={(e) => props.onChange(e.target.value)}
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
  const mod = await import('../SkillsRunnerBody');
  return mod.SkillsRunnerBody;
}

/**
 * Wrap the body in a MemoryRouter so the URL-binding effect (added in
 * Phase 4 of the /skills IA restructure) has a router context to read
 * `?skill=` from / write back to. Default entry is `/skills/run`
 * matching where the runner now lives.
 */
function renderBody(Body: React.ComponentType, initialPath = '/skills/run') {
  return render(
    <MemoryRouter initialEntries={[initialPath]}>
      <Body />
    </MemoryRouter>
  );
}

// Tests ──────────────────────────────────────────────────────────────

describe('SkillsRunnerBody — saved-schedule toggle', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach((fn) => fn.mockReset());

    hoisted.listSkills.mockResolvedValue(skillsList);
    hoisted.describeSkill.mockResolvedValue(skillDescription);
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.cronList.mockResolvedValue({ result: [makeJob({ enabled: true })] });
    hoisted.cronUpdate.mockResolvedValue({ result: makeJob({ enabled: false }) });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
  });

  it('renders the toggle in the enabled state for an enabled job', async () => {
    const Body = await importBody();
    renderBody(Body);

    // Wait for skills_list to resolve and populate the dropdown.
    await waitFor(() => expect(hoisted.listSkills).toHaveBeenCalled());

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

    await waitFor(() => expect(hoisted.listSkills).toHaveBeenCalled());
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

    await waitFor(() => expect(hoisted.listSkills).toHaveBeenCalled());
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

describe('SkillsRunnerBody — per-job history viewer', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach((fn) => fn.mockReset());
    hoisted.listSkills.mockResolvedValue(skillsList);
    hoisted.describeSkill.mockResolvedValue(skillDescription);
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.cronList.mockResolvedValue({ result: [makeJob({ enabled: true })] });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [makeRun(1), makeRun(2)] } });
  });

  it('loads cron_runs and renders history rows on first toggle', async () => {
    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listSkills).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: SKILL_ID } });
    await waitFor(() => expect(hoisted.cronList).toHaveBeenCalled());

    const historyToggle = await screen.findByTestId('history-toggle-job-1');
    fireEvent.click(historyToggle);

    await waitFor(() => expect(hoisted.cronRuns).toHaveBeenCalledWith('job-1', 5));
    expect(await screen.findByTestId('history-run-job-1-1')).toBeInTheDocument();
    expect(screen.getByTestId('history-run-job-1-2')).toBeInTheDocument();
  });

  it("expands a run row to show its captured output, hides on collapse", async () => {
    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listSkills).toHaveBeenCalled());
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
      makeJob({
        id: 'job-paused',
        name: `skill-run-${SKILL_ID}-paused`,
        enabled: false,
      }),
    ];
    hoisted.cronList.mockResolvedValue({ result: jobs });

    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listSkills).toHaveBeenCalled());
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
    const rows = ['job-recent-enabled', 'job-old-enabled', 'job-paused'].map((id) =>
      screen.getByTestId(`scheduled-job-${id}`)
    );
    expect(rows[0]).toHaveAttribute('data-active', 'true');
    expect(rows[1]).toHaveAttribute('data-active', 'true');
    expect(rows[2]).toHaveAttribute('data-active', 'false');
    // Confirm DOM order by walking the parent's children.
    const parent = rows[0].parentElement!;
    const cardChildren = Array.from(parent.children).filter((el) =>
      el.getAttribute('data-testid')?.startsWith('scheduled-job-job-')
    );
    expect(cardChildren.map((el) => el.getAttribute('data-testid'))).toEqual([
      'scheduled-job-job-recent-enabled',
      'scheduled-job-job-old-enabled',
      'scheduled-job-job-paused',
    ]);
  });

  it('does not show an Active badge when no schedules are enabled', async () => {
    hoisted.cronList.mockResolvedValue({
      result: [makeJob({ enabled: false })],
    });
    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listSkills).toHaveBeenCalled());
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
    await waitFor(() => expect(hoisted.listSkills).toHaveBeenCalled());
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

describe('SkillsRunnerBody — SmartIssuePicker conditional mount', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach((fn) => fn.mockReset());
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.cronList.mockResolvedValue({ result: [] });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
  });

  it('renders SmartIssuePicker when the picked skill is dev-workflow', async () => {
    hoisted.listSkills.mockResolvedValue([{ id: 'dev-workflow', name: 'Dev Workflow' }]);
    hoisted.describeSkill.mockResolvedValue({
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
    await waitFor(() => expect(hoisted.listSkills).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'dev-workflow' } });

    expect(await screen.findByTestId('smart-issue-picker-stub')).toBeInTheDocument();
    // The four managed inputs should NOT appear as plain text fields
    // — they're driven by the picker. We probe one of them.
    expect(screen.queryByLabelText(/target_branch/)).not.toBeInTheDocument();
  });

  it('does NOT render SmartIssuePicker for generic skills', async () => {
    hoisted.listSkills.mockResolvedValue([
      { id: 'github-issue-crusher', name: 'GitHub Issue Crusher' },
    ]);
    hoisted.describeSkill.mockResolvedValue({
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
    await waitFor(() => expect(hoisted.listSkills).toHaveBeenCalled());
    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'github-issue-crusher' } });

    await waitFor(() => expect(hoisted.describeSkill).toHaveBeenCalled());
    expect(screen.queryByTestId('smart-issue-picker-stub')).not.toBeInTheDocument();
    // The generic schema-driven repo field IS rendered via the
    // existing RepoPicker stub.
    expect(await screen.findByTestId('repo-picker-stub')).toBeInTheDocument();
  });
});

// ── Phase 4: URL ?skill= preselect binding ───────────────────────────

describe('SkillsRunnerBody — URL ?skill= preselect', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach((fn) => fn.mockReset());
    hoisted.listSkills.mockResolvedValue([
      { id: 'dev-workflow', name: 'Dev Workflow' },
      { id: 'github-issue-crusher', name: 'GitHub Issue Crusher' },
    ]);
    hoisted.describeSkill.mockResolvedValue({
      id: 'dev-workflow',
      name: 'Dev Workflow',
      when_to_use: 'Autonomous developer.',
      inputs: [],
    });
    hoisted.recentRuns.mockResolvedValue([]);
    hoisted.cronList.mockResolvedValue({ result: [] });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
  });

  it('pre-selects the skill from the ?skill= query on mount', async () => {
    const Body = await importBody();
    renderBody(Body, '/skills/run?skill=dev-workflow');

    await waitFor(() => expect(hoisted.listSkills).toHaveBeenCalled());

    // The picker should already be pointing at dev-workflow without any
    // user interaction. We assert this two ways: (a) the <select>'s
    // value matches, and (b) describeSkill was fetched for it.
    const select = (await screen.findByLabelText(
      'settings.skillsRunner.skill'
    )) as HTMLSelectElement;
    expect(select.value).toBe('dev-workflow');
    await waitFor(() =>
      expect(hoisted.describeSkill).toHaveBeenCalledWith('dev-workflow')
    );
  });

  it('does not preselect when no ?skill= is present', async () => {
    const Body = await importBody();
    renderBody(Body, '/skills/run');

    await waitFor(() => expect(hoisted.listSkills).toHaveBeenCalled());
    const select = (await screen.findByLabelText(
      'settings.skillsRunner.skill'
    )) as HTMLSelectElement;
    expect(select.value).toBe('');
    expect(hoisted.describeSkill).not.toHaveBeenCalled();
  });

  it('ignores ?skill= when the value is not in the skills_list (picker stays empty, describeSkill called once with empty=never)', async () => {
    // ?skill=unknown-skill is treated as best-effort: we set the state
    // but the picker shows "Select a skill" since the option isn't in
    // the list. The describe call IS attempted (we don't pre-filter
    // against the catalog) — but the cancellation effect tears it
    // down if the value never resolves to a real skill.
    hoisted.describeSkill.mockRejectedValue(new Error('unknown skill'));
    const Body = await importBody();
    renderBody(Body, '/skills/run?skill=does-not-exist');

    await waitFor(() => expect(hoisted.listSkills).toHaveBeenCalled());
    await waitFor(() =>
      expect(hoisted.describeSkill).toHaveBeenCalledWith('does-not-exist')
    );
    // The dropdown value won't render as an option (not in the list),
    // so its current value normalises to '' visually — but the state
    // we care about is that the error surfaces, not crashes.
    expect(
      await screen.findByText(/settings.skillsRunner.error.describe/)
    ).toBeInTheDocument();
  });
});

// ── Phase 5: Run Now flow ────────────────────────────────────────────
//
// Exercises handleRun → buildInputsPayload (lines 167-201), missing-
// required validation (lines 415-429), and the run-result render paths
// (lines 441-452).

describe('SkillsRunnerBody — Run Now flow', () => {
  beforeEach(() => {
    Object.values(hoisted).forEach((fn) => fn.mockReset());
    hoisted.listSkills.mockResolvedValue([{ id: 'pr-review-shepherd', name: 'PR Review Shepherd' }]);
    hoisted.describeSkill.mockResolvedValue({
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
    hoisted.cronList.mockResolvedValue({ result: [] });
    hoisted.cronRuns.mockResolvedValue({ result: { runs: [] } });
    hoisted.runSkill.mockResolvedValue({
      run_id: 'run-abc',
      skill_id: 'pr-review-shepherd',
      log: '/tmp/run-abc.log',
    });
  });

  it('Run Now button is disabled while required fields are empty', async () => {
    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listSkills).toHaveBeenCalled());

    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'pr-review-shepherd' } });
    await waitFor(() => expect(hoisted.describeSkill).toHaveBeenCalledWith('pr-review-shepherd'));

    // Run Now button should be disabled when required field is empty
    const runBtn = await screen.findByText('settings.skillsRunner.runNow');
    expect(runBtn.closest('button')).toBeDisabled();
    expect(hoisted.runSkill).not.toHaveBeenCalled();
  });

  it('calls runSkill with built payload when required fields are filled', async () => {
    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listSkills).toHaveBeenCalled());

    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'pr-review-shepherd' } });
    await waitFor(() => expect(hoisted.describeSkill).toHaveBeenCalled());

    // Fill the repo input (rendered as a RepoPicker stub <input>)
    const repoInput = await screen.findByTestId('repo-picker-stub');
    fireEvent.change(repoInput, { target: { value: 'owner/myrepo' } });

    // Wait for button to become enabled (state update after required field filled)
    const runBtn = await screen.findByText('settings.skillsRunner.runNow');
    await waitFor(() => expect(runBtn.closest('button')).not.toBeDisabled());
    fireEvent.click(runBtn.closest('button')!);

    await waitFor(() =>
      expect(hoisted.runSkill).toHaveBeenCalledWith(
        'pr-review-shepherd',
        expect.objectContaining({ repo: 'owner/myrepo' })
      )
    );
  });

  it('surfaces error when runSkill rejects', async () => {
    hoisted.runSkill.mockRejectedValue(new Error('backend error'));
    const Body = await importBody();
    renderBody(Body);
    await waitFor(() => expect(hoisted.listSkills).toHaveBeenCalled());

    const select = screen.getByLabelText('settings.skillsRunner.skill') as HTMLSelectElement;
    fireEvent.change(select, { target: { value: 'pr-review-shepherd' } });
    await waitFor(() => expect(hoisted.describeSkill).toHaveBeenCalled());

    const repoInput = await screen.findByTestId('repo-picker-stub');
    fireEvent.change(repoInput, { target: { value: 'owner/myrepo' } });

    const runBtn = await screen.findByText('settings.skillsRunner.runNow');
    await waitFor(() => expect(runBtn.closest('button')).not.toBeDisabled());
    fireEvent.click(runBtn.closest('button')!);

    await waitFor(() =>
      expect(screen.getByTestId('skill-run-error')).toBeInTheDocument()
    );
  });
});

