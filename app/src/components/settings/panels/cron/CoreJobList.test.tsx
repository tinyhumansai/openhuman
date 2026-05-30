import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, test, vi } from 'vitest';

import type { CoreCronJob, CoreCronRun } from '../../../../utils/tauriCommands';
import CoreJobList from './CoreJobList';

vi.mock('../../../../lib/i18n/I18nContext', () => ({
  useT: () => ({
    t: (key: string) =>
      ({
        'common.enabled': 'Enabled',
        'common.remove': 'Remove',
        'settings.cron.jobs.desc': 'Manage cron jobs',
        'settings.cron.jobs.edit': 'Edit',
        'settings.cron.jobs.lastStatus': 'Last status',
        'settings.cron.jobs.nextRun': 'Next run',
        'settings.cron.jobs.pause': 'Pause',
        'settings.cron.jobs.recentRuns': 'Recent runs',
        'settings.cron.jobs.saving': 'Saving…',
        'settings.cron.jobs.schedule': 'Schedule',
        'settings.cron.jobs.title': 'Scheduled Jobs',
        'settings.cron.jobs.viewRuns': 'View Runs',
        'subconscious.runNow': 'Run Now',
      })[key] ?? key,
  }),
}));

const job: CoreCronJob = {
  id: 'morning_briefing',
  expression: '0 9 * * *',
  schedule: { kind: 'cron', expr: '0 9 * * *' },
  command: 'brief',
  name: 'Morning Briefing',
  job_type: 'agent',
  session_target: 'isolated',
  enabled: true,
  delivery: { mode: 'origin', best_effort: true },
  delete_after_run: false,
  created_at: '2026-05-18T00:00:00.000Z',
  next_run: '2026-05-18T09:00:00.000Z',
  last_status: 'ok',
};

const run: CoreCronRun = {
  id: 42,
  job_id: job.id,
  started_at: '2026-05-18T09:00:00.000Z',
  finished_at: '2026-05-18T09:00:05.000Z',
  status: 'success',
};

function renderList() {
  return render(
    <CoreJobList
      loading={false}
      coreJobs={[job]}
      coreRunsByJob={{ [job.id]: [run] }}
      coreBusyKey={null}
      onToggleCoreJob={vi.fn()}
      onRunCoreJob={vi.fn()}
      onLoadCoreRuns={vi.fn()}
      onRemoveCoreJob={vi.fn()}
    />
  );
}

describe('CoreJobList stable test hooks', () => {
  test('renders deterministic data-testid hooks for the row and row actions', () => {
    renderList();

    expect(screen.getByTestId('cron-job-row-morning_briefing')).toBeInTheDocument();
    expect(screen.getByTestId('cron-job-toggle-morning_briefing')).toHaveTextContent('Pause');
    expect(screen.getByTestId('cron-job-run-morning_briefing')).toHaveTextContent('Run Now');
    expect(screen.getByTestId('cron-job-view-runs-morning_briefing')).toHaveTextContent(
      'View Runs'
    );
    expect(screen.getByTestId('cron-job-remove-morning_briefing')).toHaveTextContent('Remove');
    expect(screen.getByTestId('cron-job-runs-morning_briefing')).toHaveTextContent('success');
  });

  test('edit button absent when onEditCoreJob prop is not provided', () => {
    renderList();
    expect(screen.queryByTestId('cron-job-edit-morning_briefing')).not.toBeInTheDocument();
  });

  test('edit button present and invokes callback with job when onEditCoreJob is provided', () => {
    const onEditCoreJob = vi.fn();
    render(
      <CoreJobList
        loading={false}
        coreJobs={[job]}
        coreRunsByJob={{ [job.id]: [run] }}
        coreBusyKey={null}
        onToggleCoreJob={vi.fn()}
        onRunCoreJob={vi.fn()}
        onLoadCoreRuns={vi.fn()}
        onRemoveCoreJob={vi.fn()}
        onEditCoreJob={onEditCoreJob}
      />
    );

    const editBtn = screen.getByTestId('cron-job-edit-morning_briefing');
    expect(editBtn).toBeInTheDocument();
    expect(editBtn).toHaveTextContent('Edit');

    fireEvent.click(editBtn);
    expect(onEditCoreJob).toHaveBeenCalledOnce();
    expect(onEditCoreJob).toHaveBeenCalledWith(job);
  });

  test('toggle button shows saving label when coreBusyKey targets the toggle', () => {
    render(
      <CoreJobList
        loading={false}
        coreJobs={[job]}
        coreRunsByJob={{}}
        coreBusyKey={`core-toggle:${job.id}`}
        onToggleCoreJob={vi.fn()}
        onRunCoreJob={vi.fn()}
        onLoadCoreRuns={vi.fn()}
        onRemoveCoreJob={vi.fn()}
      />
    );
    expect(screen.getByTestId(`cron-job-toggle-${job.id}`)).toHaveTextContent('Saving…');
  });
});
