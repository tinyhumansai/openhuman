import { History, Pause, Pencil, Play, Trash2, Zap } from 'lucide-react';
import { Fragment, type ReactNode, useMemo, useState } from 'react';

import { useT } from '../../../../lib/i18n/I18nContext';
import { intlLocale } from '../../../../lib/i18n/intlLocale';
import type { CoreCronJob, CoreCronRun } from '../../../../utils/tauriCommands';
import Badge, { type BadgeVariant } from '../../../ui/Badge';
import Button from '../../../ui/Button';
import DataTable, { type DataTableColumn } from '../../../ui/DataTable';
import EmptyState from '../../../ui/EmptyState';
import { TableCell, TableRow } from '../../../ui/Table';

interface CoreJobListProps {
  loading: boolean;
  coreJobs: CoreCronJob[];
  coreRunsByJob: Record<string, CoreCronRun[]>;
  coreBusyKey: string | null;
  onToggleCoreJob: (job: CoreCronJob) => void;
  onRunCoreJob: (jobId: string) => void;
  onLoadCoreRuns: (jobId: string) => void;
  onRemoveCoreJob: (jobId: string) => void;
  /** Optional: when provided, an Edit button is rendered per row. */
  onEditCoreJob?: (job: CoreCronJob) => void;
  /** Card title / description / header actions (New job, Refresh). */
  title?: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  /** Rendered above the rows (load / action failures). */
  error?: ReactNode;
}

/** Map a free-form core status ("ok", "error", "failed", …) to a chip tone. */
function statusVariant(status: string): BadgeVariant {
  const s = status.toLowerCase();
  if (/(ok|success|succeeded|completed|done)/.test(s)) return 'success';
  if (/(err|fail|timeout|panic)/.test(s)) return 'danger';
  if (/(skip|cancel|warn)/.test(s)) return 'warning';
  return 'neutral';
}

/** Date over time, in the app's active locale; raw value when unparsable. */
const DateTimeCell = ({ value, locale }: { value: string; locale: string }) => {
  const ts = Date.parse(value);
  if (Number.isNaN(ts)) return <span className="text-content-muted">{value}</span>;
  const date = new Date(ts);
  return (
    <span className="flex flex-col leading-tight" title={date.toLocaleString(locale)}>
      <span className="text-content">{date.toLocaleDateString(locale)}</span>
      <span className="text-xs text-content-muted">{date.toLocaleTimeString(locale)}</span>
    </span>
  );
};

const STATUS_FILTER_VALUES = ['enabled', 'paused'] as const;

/**
 * The scheduled-jobs table: one row per core cron job with its schedule, next
 * run, last status and row actions. "Runs" loads the job's recent history into
 * a full-width sub-row under it.
 */
const CoreJobList = ({
  loading,
  coreJobs,
  coreRunsByJob,
  coreBusyKey,
  onToggleCoreJob,
  onRunCoreJob,
  onLoadCoreRuns,
  onRemoveCoreJob,
  onEditCoreJob,
  title,
  description,
  actions,
  error,
}: CoreJobListProps) => {
  const { t, locale } = useT();
  const intl = intlLocale(locale);
  const [query, setQuery] = useState('');
  const [selectedStatus, setSelectedStatus] = useState<ReadonlySet<string>>(new Set());

  const scheduleText = (job: CoreCronJob) =>
    job.schedule.kind === 'cron'
      ? job.schedule.expr
      : job.schedule.kind === 'every'
        ? t('settings.cron.jobs.scheduleEvery').replace('{ms}', String(job.schedule.every_ms))
        : t('settings.cron.jobs.scheduleAt').replace('{time}', job.schedule.at);

  const filteredJobs = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return coreJobs.filter(job => {
      if (selectedStatus.size > 0 && !selectedStatus.has(job.enabled ? 'enabled' : 'paused')) {
        return false;
      }
      if (!needle) return true;
      return [job.name ?? '', job.id, job.command ?? '', job.prompt ?? ''].some(value =>
        value.toLowerCase().includes(needle)
      );
    });
  }, [coreJobs, query, selectedStatus]);

  const toggleLabel = (job: CoreCronJob) =>
    coreBusyKey === `core-toggle:${job.id}`
      ? t('settings.cron.jobs.saving')
      : job.enabled
        ? t('settings.cron.jobs.pause')
        : t('settings.cron.jobs.resume');
  const runLabel = (jobId: string) =>
    coreBusyKey === `core-run:${jobId}`
      ? t('settings.cron.jobs.runningNow')
      : t('settings.cron.jobs.runNow');
  const runsLabel = (jobId: string) =>
    coreBusyKey === `core-runs:${jobId}`
      ? t('settings.cron.jobs.loadingRuns')
      : t('settings.cron.jobs.viewRuns');
  const removeLabel = (jobId: string) =>
    coreBusyKey === `core-remove:${jobId}` ? t('settings.cron.jobs.removing') : t('common.remove');

  /** Compact icon button; the (busy-aware) label rides aria-label + tooltip. */
  const iconAction = (
    label: string,
    icon: ReactNode,
    onClick: () => void,
    testId: string,
    opts: { disabled?: boolean; danger?: boolean } = {}
  ) => (
    <Button
      type="button"
      variant="tertiary"
      tone={opts.danger ? 'danger' : undefined}
      size="sm"
      aria-label={label}
      title={label}
      data-testid={testId}
      disabled={opts.disabled}
      onClick={event => {
        event.stopPropagation();
        onClick();
      }}
      className="h-8 w-8 px-0">
      {icon}
    </Button>
  );

  const columns: DataTableColumn<CoreCronJob>[] = [
    {
      id: 'job',
      header: t('settings.cron.jobs.columnJob'),
      className: 'w-full max-w-0',
      cell: job => (
        <div className="min-w-0">
          <p className="truncate font-medium text-content" title={job.name || job.id}>
            {job.name || job.id}
          </p>
          <p className="truncate font-mono text-[11px] text-content-faint" title={job.id}>
            {job.id}
          </p>
        </div>
      ),
    },
    {
      id: 'schedule',
      header: t('settings.cron.jobs.schedule'),
      className: 'w-px whitespace-nowrap',
      cell: job => <span className="font-mono text-xs text-content">{scheduleText(job)}</span>,
    },
    {
      id: 'next',
      header: t('settings.cron.jobs.nextRun'),
      className: 'w-px whitespace-nowrap tabular-nums',
      cell: job => <DateTimeCell value={job.next_run} locale={intl} />,
    },
    {
      id: 'last',
      header: t('settings.cron.jobs.lastStatus'),
      className: 'w-px whitespace-nowrap',
      cell: job =>
        job.last_status ? (
          <Badge variant={statusVariant(job.last_status)}>{job.last_status}</Badge>
        ) : (
          <span className="text-content-faint">—</span>
        ),
    },
    {
      id: 'state',
      header: t('settings.cron.jobs.columnState'),
      className: 'w-px whitespace-nowrap',
      cell: job => (
        <Badge variant={job.enabled ? 'success' : 'neutral'}>
          {job.enabled ? t('common.enabled') : t('settings.cron.jobs.paused')}
        </Badge>
      ),
    },
    {
      id: 'actions',
      header: <span className="sr-only">{t('settings.cron.jobs.columnActions')}</span>,
      align: 'right',
      className: 'w-px whitespace-nowrap',
      cell: job => (
        <div className="flex items-center justify-end gap-0.5">
          {iconAction(
            toggleLabel(job),
            job.enabled ? (
              <Pause className="h-4 w-4" aria-hidden />
            ) : (
              <Play className="h-4 w-4" aria-hidden />
            ),
            () => onToggleCoreJob(job),
            `cron-job-toggle-${job.id}`,
            { disabled: coreBusyKey === `core-toggle:${job.id}` }
          )}
          {iconAction(
            runLabel(job.id),
            <Zap className="h-4 w-4" aria-hidden />,
            () => onRunCoreJob(job.id),
            `cron-job-run-${job.id}`,
            { disabled: coreBusyKey === `core-run:${job.id}` }
          )}
          {iconAction(
            runsLabel(job.id),
            <History className="h-4 w-4" aria-hidden />,
            () => onLoadCoreRuns(job.id),
            `cron-job-view-runs-${job.id}`,
            { disabled: coreBusyKey === `core-runs:${job.id}` }
          )}
          {onEditCoreJob &&
            iconAction(
              t('settings.cron.jobs.edit'),
              <Pencil className="h-4 w-4" aria-hidden />,
              () => onEditCoreJob(job),
              `cron-job-edit-${job.id}`
            )}
          {iconAction(
            removeLabel(job.id),
            <Trash2 className="h-4 w-4" aria-hidden />,
            () => onRemoveCoreJob(job.id),
            `cron-job-remove-${job.id}`,
            { disabled: coreBusyKey === `core-remove:${job.id}`, danger: true }
          )}
        </div>
      ),
    },
  ];

  const renderRuns = (job: CoreCronJob, runs: CoreCronRun[]) => (
    <TableRow className="hover:bg-transparent">
      <TableCell colSpan={columns.length} className="bg-surface-muted py-3">
        <div data-testid={`cron-job-runs-${job.id}`} className="space-y-2">
          <p className="text-xs font-medium text-content-muted">
            {t('settings.cron.jobs.recentRuns')}
          </p>
          <ul className="flex flex-wrap gap-2">
            {runs.map(run => {
              const finishedAt = new Date(run.finished_at).toLocaleString(intl);
              // Split on the placeholders (rather than assuming
              // "{status} at {time}" order) so a locale that reorders the
              // phrase still renders correctly.
              const parts = t('settings.cron.jobs.runFinishedAt').split(/(\{status\}|\{time\})/g);
              return (
                <li key={run.id}>
                  <Badge variant={statusVariant(run.status)} title={run.output ?? undefined}>
                    {parts.map((part, index) =>
                      part === '{status}' ? (
                        <Fragment key={index}>{run.status}</Fragment>
                      ) : part === '{time}' ? (
                        <span key={index} className="font-normal text-content-muted">
                          {finishedAt}
                        </span>
                      ) : (
                        <Fragment key={index}>{part}</Fragment>
                      )
                    )}
                  </Badge>
                </li>
              );
            })}
          </ul>
        </div>
      </TableCell>
    </TableRow>
  );

  return (
    <DataTable<CoreCronJob>
      title={title}
      description={description}
      actions={actions}
      error={error}
      columns={columns}
      rows={filteredJobs}
      rowKey={job => job.id}
      renderRow={job => {
        const runs = coreRunsByJob[job.id] ?? [];
        return (
          <Fragment key={job.id}>
            <TableRow data-testid={`cron-job-row-${job.id}`}>
              {columns.map(column => (
                <TableCell
                  key={column.id}
                  className={
                    column.align === 'right'
                      ? `${column.className ?? ''} text-right`
                      : column.className
                  }>
                  {column.cell?.(job)}
                </TableCell>
              ))}
            </TableRow>
            {runs.length > 0 && renderRuns(job, runs)}
          </Fragment>
        );
      }}
      search={{
        value: query,
        onChange: setQuery,
        placeholder: t('settings.cron.jobs.searchPlaceholder'),
        testId: 'cron-jobs-search',
      }}
      filters={[
        {
          id: 'state',
          label: t('settings.cron.jobs.columnState'),
          options: STATUS_FILTER_VALUES.map(value => ({
            value,
            label: value === 'enabled' ? t('common.enabled') : t('settings.cron.jobs.paused'),
          })),
          selected: selectedStatus,
          onChange: setSelectedStatus,
          testId: 'cron-jobs-state-filter',
        },
      ]}
      pagination={{ pageSize: 25 }}
      loading={loading}
      loadingLabel={t('settings.cron.jobs.loading')}
      empty={<EmptyState label={t('settings.cron.jobs.empty')} />}
      ariaLabel={t('cron.scheduledJobs')}
      testId="cron-jobs-table"
    />
  );
};

export default CoreJobList;
