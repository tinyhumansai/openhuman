/**
 * SkillsDashboard — Phase 3 coverage.
 *
 * Covers:
 *  - empty state (no skill-* cron jobs found) renders the empty card +
 *    Run-a-Skill CTA.
 *  - non-empty state groups jobs by skill_id and renders one card per
 *    skill, with the schedule rendered through cronToHuman.
 *  - card click navigates to /skills/run?skill=<id>.
 *  - toggle round-trip: clicking flips enabled via openhumanCronUpdate
 *    and reloads via openhumanCronList; aria-checked reflects the new
 *    state.
 *  - load error renders the error card with a retry button.
 *  - jobs that don't start with `skill-run-` are filtered out (so a
 *    user's unrelated cron jobs don't leak onto this page).
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import SkillsDashboard from './SkillsDashboard';

const stableT = (key: string) => key;
vi.mock('../lib/i18n/I18nContext', () => ({ useT: () => ({ t: stableT }) }));

const hoisted = vi.hoisted(() => ({ cronList: vi.fn(), cronUpdate: vi.fn() }));

vi.mock('../utils/tauriCommands/cron', () => ({
  openhumanCronList: hoisted.cronList,
  openhumanCronUpdate: hoisted.cronUpdate,
}));

const renderDashboard = () =>
  render(
    <MemoryRouter initialEntries={['/skills']}>
      <Routes>
        <Route path="/skills" element={<SkillsDashboard />} />
        <Route
          path="/skills/run"
          element={<div data-testid="runner-landed">{window.location.hash}</div>}
        />
        <Route path="/skills/new" element={<div data-testid="new-landed">new</div>} />
      </Routes>
    </MemoryRouter>
  );

function makeJob(overrides: Partial<Record<string, unknown>> = {}) {
  return {
    id: 'job-1',
    expression: '*/30 * * * *',
    schedule: { kind: 'cron', expr: '*/30 * * * *' },
    command: '',
    prompt: null,
    name: 'skill-run-dev-workflow-repo=owner-repo',
    job_type: 'agent',
    session_target: 'isolated',
    model: null,
    enabled: true,
    delivery: { mode: 'proactive', best_effort: true },
    delete_after_run: false,
    created_at: '2026-05-20T10:00:00Z',
    next_run: '2026-05-29T03:00:00Z',
    last_run: '2026-05-29T02:30:00Z',
    last_status: 'ok',
    last_output: null,
    ...overrides,
  };
}

describe('SkillsDashboard', () => {
  beforeEach(() => {
    hoisted.cronList.mockReset();
    hoisted.cronUpdate.mockReset();
  });

  it('renders the empty state when no skill-run-* jobs exist', async () => {
    hoisted.cronList.mockResolvedValue({ result: [] });
    renderDashboard();

    await screen.findByTestId('skills-dashboard-empty');
    expect(screen.getByText('skills.dashboard.emptyTitle')).toBeInTheDocument();

    // CTA → /skills/run.
    fireEvent.click(screen.getByTestId('skills-dashboard-empty-cta'));
    expect(screen.getByTestId('runner-landed')).toBeInTheDocument();
  });

  it('surfaces skill-run-* AND legacy dev-workflow-* crons, drops unrelated ones', async () => {
    // The legacy `dev-workflow-<repo>` naming (written by
    // DevWorkflowPanel before the unified `skill-run-` convention)
    // must surface on the dashboard so users can toggle / edit the
    // dev-workflow schedule they already set up. Anything that doesn't
    // match either prefix (memory-tree maintenance, etc.) stays out.
    hoisted.cronList.mockResolvedValue({
      result: [
        makeJob({ id: 'j-modern', name: 'skill-run-github-issue-crusher-repo=foo-bar' }),
        makeJob({ id: 'j-legacy', name: 'dev-workflow-tinyhumansai-openhuman' }),
        makeJob({ id: 'j-unrelated', name: 'memory-tree-maintenance' }),
      ],
    });
    renderDashboard();

    await screen.findByTestId('skill-card-github-issue-crusher');
    expect(screen.queryByTestId('skills-dashboard-empty')).not.toBeInTheDocument();
    expect(screen.queryAllByTestId('skill-card-github-issue-crusher')).toHaveLength(1);
    // Legacy dev-workflow naming is mapped to skill_id 'dev-workflow'
    // and gets its own card.
    expect(screen.queryAllByTestId('skill-card-dev-workflow')).toHaveLength(1);
    // Unrelated cron doesn't get a card.
    expect(screen.queryAllByTestId('skill-card-memory-tree-maintenance')).toHaveLength(0);
  });

  it('groups multiple jobs for the same skill into one card with an ×N badge', async () => {
    hoisted.cronList.mockResolvedValue({
      result: [
        makeJob({ id: 'a', name: 'skill-run-dev-workflow-repo=owner-foo' }),
        makeJob({ id: 'b', name: 'skill-run-dev-workflow-repo=owner-bar', enabled: false }),
      ],
    });
    renderDashboard();

    await screen.findByTestId('skill-card-dev-workflow');
    // Multi-job badge.
    expect(screen.getByText('×2')).toBeInTheDocument();
    // Picks the enabled job as primary → toggle aria-checked is true.
    expect(screen.getByTestId('skill-card-dev-workflow-toggle')).toHaveAttribute(
      'aria-checked',
      'true'
    );
  });

  it('renders the schedule via cronToHuman', async () => {
    hoisted.cronList.mockResolvedValue({
      result: [makeJob({ name: 'skill-run-github-issue-crusher-x=1' })],
    });
    renderDashboard();

    await screen.findByTestId('skill-card-github-issue-crusher');
    // `*/30 * * * *` → "Every 30 minutes".
    expect(screen.getByText('Every 30 minutes')).toBeInTheDocument();
  });

  it('clicking a card navigates to /skills/run?skill=<id>', async () => {
    hoisted.cronList.mockResolvedValue({
      result: [makeJob({ name: 'skill-run-dev-workflow-repo=x' })],
    });
    renderDashboard();

    const card = await screen.findByTestId('skill-card-dev-workflow-open');
    fireEvent.click(card);
    expect(screen.getByTestId('runner-landed')).toBeInTheDocument();
  });

  it('header CTAs navigate to /skills/new and /skills/run', async () => {
    hoisted.cronList.mockResolvedValue({ result: [] });
    const { unmount } = renderDashboard();
    await screen.findByTestId('skills-dashboard-empty');
    fireEvent.click(screen.getByTestId('skills-dashboard-create'));
    expect(screen.getByTestId('new-landed')).toBeInTheDocument();
    unmount();

    hoisted.cronList.mockResolvedValue({ result: [] });
    renderDashboard();
    await screen.findByTestId('skills-dashboard-empty');
    fireEvent.click(screen.getByTestId('skills-dashboard-run'));
    expect(screen.getByTestId('runner-landed')).toBeInTheDocument();
  });

  it('toggle flips enabled via openhumanCronUpdate and reloads the list', async () => {
    let listCalls = 0;
    hoisted.cronList.mockImplementation(async () => {
      listCalls += 1;
      // First call: enabled=true. Second call (after update): enabled=false.
      return {
        result: [
          makeJob({ id: 'j-1', name: 'skill-run-dev-workflow-repo=x', enabled: listCalls === 1 }),
        ],
      };
    });
    hoisted.cronUpdate.mockResolvedValue({
      result: makeJob({ id: 'j-1', name: 'skill-run-dev-workflow-repo=x', enabled: false }),
    });

    renderDashboard();
    const toggle = await screen.findByTestId('skill-card-dev-workflow-toggle');
    expect(toggle).toHaveAttribute('aria-checked', 'true');

    fireEvent.click(toggle);

    await waitFor(() => {
      expect(hoisted.cronUpdate).toHaveBeenCalledWith('j-1', { enabled: false });
    });
    // List reloaded (init + post-toggle = 2 calls).
    await waitFor(() => {
      expect(listCalls).toBe(2);
    });
    // aria-checked flipped after reload.
    await waitFor(() => {
      expect(screen.getByTestId('skill-card-dev-workflow-toggle')).toHaveAttribute(
        'aria-checked',
        'false'
      );
    });
  });

  it('renders an error card with retry when cronList fails', async () => {
    hoisted.cronList
      .mockRejectedValueOnce(new Error('rpc down'))
      .mockResolvedValueOnce({ result: [] });
    renderDashboard();

    await screen.findByTestId('skills-dashboard-error');
    expect(screen.getByText(/rpc down/)).toBeInTheDocument();

    fireEvent.click(screen.getByText('common.retry'));
    await screen.findByTestId('skills-dashboard-empty');
  });
});
