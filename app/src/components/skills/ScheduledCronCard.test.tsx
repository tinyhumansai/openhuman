/**
 * ScheduledCronCard — Phase 1 coverage.
 *
 * The card is presentation-only: callers drive `onToggle` and `onClick`,
 * the card just renders a polished surface for one CoreCronJob. Tests
 * lock down the contract:
 *
 *  - toggle round-trip: click → `onToggle(!enabled)`.
 *  - whole-card click → `onClick()` (only when `onClick` is provided).
 *  - schedule rendered via cronToHuman for the common preset.
 *  - last_run + next_run + last_status badge surface in the meta row.
 *  - badgeCount renders `×N` only when > 1.
 *  - activeBadge renders the `★ Active` pill when set.
 *  - busy disables the toggle.
 *  - actions slot lives next to the toggle and doesn't bubble to onClick.
 *  - testIdRoot drives the emitted testids so consumers can target
 *    parts of the card.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const stableT = (key: string) => key;
vi.mock('../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: stableT }) }));

import type { CoreCronJob } from '../../utils/tauriCommands/cron';

import ScheduledCronCard from './ScheduledCronCard';

function makeJob(overrides: Partial<CoreCronJob> = {}): CoreCronJob {
  return {
    id: 'job-1',
    expression: '*/30 * * * *',
    schedule: { kind: 'cron', expr: '*/30 * * * *' },
    command: '',
    prompt: '',
    name: 'skill-run-dev-workflow-repo=owner-repo',
    job_type: 'agent',
    session_target: 'isolated',
    enabled: true,
    delivery: { mode: 'proactive', best_effort: true },
    delete_after_run: false,
    created_at: '2026-05-20T10:00:00Z',
    next_run: '2026-05-29T03:00:00Z',
    last_run: '2026-05-29T02:30:00Z',
    last_status: 'ok',
    last_output: null,
    ...overrides,
  } as CoreCronJob;
}

describe('ScheduledCronCard', () => {
  it('renders the title, cronToHuman schedule, and meta row', () => {
    render(
      <ScheduledCronCard
        job={makeJob()}
        title="dev-workflow"
        onToggle={() => undefined}
      />
    );
    expect(screen.getByTestId('scheduled-cron-job-1-title')).toHaveTextContent('dev-workflow');
    // `*/30 * * * *` → "Every 30 minutes" per cronToHuman.
    expect(screen.getByTestId('scheduled-cron-job-1-schedule')).toHaveTextContent(
      'Every 30 minutes'
    );
    // last_run + next_run labels surface.
    expect(screen.getByText(/skills.dashboard.lastRun/)).toBeInTheDocument();
    expect(screen.getByText(/skills.dashboard.nextRun/)).toBeInTheDocument();
    // last_status chip.
    expect(screen.getByTestId('scheduled-cron-job-1-last-status')).toHaveTextContent('ok');
  });

  it('defaults the heading to job.name when no title prop given', () => {
    render(<ScheduledCronCard job={makeJob()} onToggle={() => undefined} />);
    expect(screen.getByTestId('scheduled-cron-job-1-title')).toHaveTextContent(
      'skill-run-dev-workflow-repo=owner-repo'
    );
  });

  it('calls onToggle with the inverted enabled state when the switch is clicked', () => {
    const onToggle = vi.fn();
    render(<ScheduledCronCard job={makeJob({ enabled: true })} onToggle={onToggle} />);

    const toggle = screen.getByTestId('scheduled-cron-job-1-toggle');
    expect(toggle).toHaveAttribute('aria-checked', 'true');

    fireEvent.click(toggle);
    expect(onToggle).toHaveBeenCalledTimes(1);
    expect(onToggle).toHaveBeenCalledWith(false);
  });

  it('round-trips off→on when initial enabled is false', () => {
    const onToggle = vi.fn();
    render(<ScheduledCronCard job={makeJob({ enabled: false })} onToggle={onToggle} />);

    const toggle = screen.getByTestId('scheduled-cron-job-1-toggle');
    expect(toggle).toHaveAttribute('aria-checked', 'false');

    fireEvent.click(toggle);
    expect(onToggle).toHaveBeenCalledWith(true);
  });

  it('reflects busy=true by disabling the toggle', () => {
    const onToggle = vi.fn();
    render(<ScheduledCronCard job={makeJob()} onToggle={onToggle} busy />);
    const toggle = screen.getByTestId('scheduled-cron-job-1-toggle') as HTMLButtonElement;
    expect(toggle).toBeDisabled();
  });

  it('renders a clickable surface and fires onClick when provided', () => {
    const onClick = vi.fn();
    const onToggle = vi.fn();
    render(<ScheduledCronCard job={makeJob()} onClick={onClick} onToggle={onToggle} />);
    const opener = screen.getByTestId('scheduled-cron-job-1-open');
    fireEvent.click(opener);
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('does not render the clickable opener when onClick is omitted', () => {
    render(<ScheduledCronCard job={makeJob()} onToggle={() => undefined} />);
    expect(screen.queryByTestId('scheduled-cron-job-1-open')).not.toBeInTheDocument();
    // Falls back to a static row.
    expect(screen.getByTestId('scheduled-cron-job-1-row')).toBeInTheDocument();
  });

  it('clicking the toggle on a clickable card does NOT also fire onClick', () => {
    const onClick = vi.fn();
    const onToggle = vi.fn();
    render(<ScheduledCronCard job={makeJob()} onClick={onClick} onToggle={onToggle} />);
    fireEvent.click(screen.getByTestId('scheduled-cron-job-1-toggle'));
    expect(onToggle).toHaveBeenCalledTimes(1);
    expect(onClick).not.toHaveBeenCalled();
  });

  it('renders the ×N count badge when badgeCount > 1', () => {
    render(
      <ScheduledCronCard
        job={makeJob()}
        badgeCount={3}
        onToggle={() => undefined}
      />
    );
    expect(screen.getByTestId('scheduled-cron-job-1-count-badge')).toHaveTextContent('×3');
  });

  it('omits the count badge when badgeCount is 1 or absent', () => {
    const { rerender } = render(
      <ScheduledCronCard job={makeJob()} onToggle={() => undefined} />
    );
    expect(screen.queryByTestId('scheduled-cron-job-1-count-badge')).not.toBeInTheDocument();
    rerender(
      <ScheduledCronCard job={makeJob()} badgeCount={1} onToggle={() => undefined} />
    );
    expect(screen.queryByTestId('scheduled-cron-job-1-count-badge')).not.toBeInTheDocument();
  });

  it('renders the ★ Active badge when activeBadge is true', () => {
    render(
      <ScheduledCronCard job={makeJob()} activeBadge onToggle={() => undefined} />
    );
    expect(screen.getByTestId('scheduled-cron-job-1-active-badge')).toBeInTheDocument();
  });

  it('renders actions slot, and clicking an action does not bubble to onClick', () => {
    const onClick = vi.fn();
    const onAction = vi.fn();
    render(
      <ScheduledCronCard
        job={makeJob()}
        onClick={onClick}
        onToggle={() => undefined}
        actions={
          <button type="button" data-testid="action-run-now" onClick={onAction}>
            run now
          </button>
        }
      />
    );
    fireEvent.click(screen.getByTestId('action-run-now'));
    expect(onAction).toHaveBeenCalledTimes(1);
    expect(onClick).not.toHaveBeenCalled();
  });

  it('renders children below the row (used for history disclosure)', () => {
    render(
      <ScheduledCronCard job={makeJob()} onToggle={() => undefined}>
        <div data-testid="history-slot">history goes here</div>
      </ScheduledCronCard>
    );
    expect(screen.getByTestId('history-slot')).toBeInTheDocument();
  });

  it('honours custom testIdRoot so consumers can target parts of the card', () => {
    render(
      <ScheduledCronCard
        job={makeJob()}
        onToggle={() => undefined}
        testIdRoot="skill-card-dev-workflow"
      />
    );
    expect(screen.getByTestId('skill-card-dev-workflow')).toBeInTheDocument();
    expect(screen.getByTestId('skill-card-dev-workflow-toggle')).toBeInTheDocument();
  });

  it('falls back to the raw expression when schedule shape is unknown', () => {
    const job = makeJob({
      // Cast to any to feed an unrecognised schedule shape — same defensive
      // path the card relies on for legacy jobs.
      schedule: { kind: 'mystery' } as unknown as CoreCronJob['schedule'],
      expression: '*/30 * * * *',
    });
    render(<ScheduledCronCard job={job} onToggle={() => undefined} />);
    expect(screen.getByTestId('scheduled-cron-job-1-schedule')).toHaveTextContent(
      'Every 30 minutes'
    );
  });

  it('renders an `at` schedule as a localised timestamp', () => {
    const job = makeJob({
      schedule: { kind: 'at', at: '2026-05-29T12:00:00Z' } as CoreCronJob['schedule'],
    });
    render(<ScheduledCronCard job={job} onToggle={() => undefined} />);
    const schedule = screen.getByTestId('scheduled-cron-job-1-schedule');
    // The exact wallclock string is locale-dependent; just assert it
    // contains a year so we know we hit the `at` branch.
    expect(schedule.textContent).toMatch(/2026/);
  });

  it('renders an `every` schedule as a minute count', () => {
    const job = makeJob({
      schedule: { kind: 'every', every_ms: 5 * 60_000 } as CoreCronJob['schedule'],
    });
    render(<ScheduledCronCard job={job} onToggle={() => undefined} />);
    expect(screen.getByTestId('scheduled-cron-job-1-schedule')).toHaveTextContent(
      'Every 5 minutes'
    );
  });

  it('renders the raw expression as a blank fallback when no schedule is set', () => {
    const job = makeJob({
      schedule: undefined as unknown as CoreCronJob['schedule'],
      expression: '0 9 * * *',
    });
    render(<ScheduledCronCard job={job} onToggle={() => undefined} />);
    // Cronstrue-ish helper returns "Every day at 09:00" for the daily preset.
    expect(screen.getByTestId('scheduled-cron-job-1-schedule').textContent).not.toEqual('');
  });

  it('renders the data-active attribute matching enabled state', () => {
    const { rerender } = render(
      <ScheduledCronCard job={makeJob({ enabled: true })} onToggle={() => undefined} />
    );
    expect(screen.getByTestId('scheduled-cron-job-1')).toHaveAttribute('data-active', 'true');
    rerender(
      <ScheduledCronCard job={makeJob({ enabled: false })} onToggle={() => undefined} />
    );
    expect(screen.getByTestId('scheduled-cron-job-1')).toHaveAttribute('data-active', 'false');
  });
});
