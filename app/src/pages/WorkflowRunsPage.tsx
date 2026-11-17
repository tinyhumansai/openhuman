/**
 * WorkflowRunsPage — the aggregate "All runs" view: every workflow's runs across
 * the whole `flows` domain, newest first, backed by the `flows_list_all_runs`
 * core RPC. Each row links back to its workflow's canvas. Stays live via
 * {@link useFlowRunsLiveRefresh} while any listed run is still active, via a
 * lightweight silent refresh (re-fetches just the runs, not `listFlows()` too)
 * so a run doesn't sit on "Running" until the user reloads the page.
 */
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { FlowRunStatus } from '../components/flows/FlowRunStatus';
import SettingsTabbedPage from '../components/settings/layout/SettingsTabbedPage';
import DataTable, { type DataTableColumn } from '../components/ui/DataTable';
import { ErrorBanner } from '../components/ui/LoadingState';
import { TableCell, TableRow } from '../components/ui/Table';
import { useFlowRunFinished } from '../hooks/useFlowRunFinished';
import { useFlowRunsLiveRefresh } from '../hooks/useFlowRunsLiveRefresh';
import { useFlowRunsQuery } from '../hooks/useFlowRunsQuery';
import { useFlowRunStarted } from '../hooks/useFlowRunStarted';
import {
  resolveDisplayStatus,
  useRunsPendingApprovalSet,
} from '../hooks/useRunsPendingApprovalSet';
import { cn } from '../lib/cn';
import { useT } from '../lib/i18n/I18nContext';
import {
  type Flow,
  type FlowRun,
  type FlowRunStatus as FlowRunStatusValue,
  listFlows,
} from '../services/api/flowsApi';

interface RunRow {
  run: FlowRun;
  displayStatus: FlowRunStatusValue;
  name: string;
}

/** Date over time, in the user's locale; raw value when unparseable. */
const DateTimeCell = ({ value }: { value: string }) => {
  const ts = Date.parse(value);
  if (Number.isNaN(ts)) return <span>{value}</span>;
  const date = new Date(ts);
  return (
    <span className="flex flex-col leading-tight" title={date.toLocaleString()}>
      <span className="text-content">{date.toLocaleDateString()}</span>
      <span className="text-xs text-content-muted">{date.toLocaleTimeString()}</span>
    </span>
  );
};

/** Compact run duration ("850ms", "12s", "3m 04s", "1h 02m"); "—" while running. */
function formatDuration(startedAt: string, finishedAt?: string | null): string {
  if (!finishedAt) return '—';
  const ms = Date.parse(finishedAt) - Date.parse(startedAt);
  if (!Number.isFinite(ms) || ms < 0) return '—';
  if (ms < 1000) return `${ms}ms`;
  const secs = Math.round(ms / 1000);
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ${String(secs % 60).padStart(2, '0')}s`;
  return `${Math.floor(mins / 60)}h ${String(mins % 60).padStart(2, '0')}m`;
}

export default function WorkflowRunsPage() {
  const { t } = useT();
  const navigate = useNavigate();
  const { runs, loading, error, refreshSilently } = useFlowRunsQuery({ scope: { kind: 'all' } });
  const [flowNames, setFlowNames] = useState<Record<string, string>>({});
  const [flowNamesLoading, setFlowNamesLoading] = useState(true);
  const [flowNamesError, setFlowNamesError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setFlowNamesLoading(true);
    setFlowNamesError(null);
    listFlows()
      .then(flows => {
        if (cancelled) return;
        const names: Record<string, string> = {};
        flows.forEach((flow: Flow) => {
          names[flow.id] = flow.name;
        });
        setFlowNames(names);
      })
      .catch(nameError => {
        if (cancelled) return;
        setFlowNamesError(nameError instanceof Error ? nameError.message : String(nameError));
      })
      .finally(() => {
        if (!cancelled) setFlowNamesLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, []);

  const handleRunFinished = useCallback(() => void refreshSilently(), [refreshSilently]);
  const handleRunStarted = useCallback(() => void refreshSilently(), [refreshSilently]);
  useFlowRunsLiveRefresh(runs, refreshSilently);
  useFlowRunFinished(handleRunFinished);
  // Unconditional (unlike useFlowRunsLiveRefresh, which is gated on an
  // already-active run) — fills the empty-list gap ("No runs yet") that hook
  // can't reach, so the very first run across any flow shows up as "Running"
  // instantly instead of waiting for a manual refresh (issue B35). No
  // `flowId` filter — this is the flow-agnostic "all runs" page.
  useFlowRunStarted(handleRunStarted);
  const pendingRunIds = useRunsPendingApprovalSet(runs);
  const pageLoading = loading || flowNamesLoading;
  const pageError = error ?? flowNamesError;

  const statusLabel = (status: FlowRunStatusValue) =>
    t(`flows.allRuns.status.${status}`, status.replace(/_/g, ' '));

  const [query, setQuery] = useState('');
  const [selectedStatus, setSelectedStatus] = useState<ReadonlySet<string>>(new Set());

  const rows = useMemo(
    () =>
      runs.map(run => ({
        run,
        displayStatus: resolveDisplayStatus(run, pendingRunIds),
        name: flowNames[run.flow_id] ?? t('flows.allRuns.unknownWorkflow'),
      })),
    [runs, pendingRunIds, flowNames, t]
  );

  const statusOptions = useMemo(
    () => Array.from(new Set(rows.map(row => row.displayStatus))),
    [rows]
  );

  const filteredRows = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return rows.filter(row => {
      if (selectedStatus.size > 0 && !selectedStatus.has(row.displayStatus)) return false;
      if (!needle) return true;
      return (
        row.name.toLowerCase().includes(needle) ||
        (row.run.error ?? '').toLowerCase().includes(needle)
      );
    });
  }, [rows, query, selectedStatus]);

  const openRun = (row: RunRow) => navigate(`/flows/${row.run.flow_id}`);

  const columns: DataTableColumn<RunRow>[] = [
    {
      // Status first: the row's first `span` is the status chip.
      id: 'status',
      header: t('flows.allRuns.columnStatus'),
      className: 'w-px whitespace-nowrap',
      cell: row => (
        <FlowRunStatus status={row.displayStatus} label={statusLabel(row.displayStatus)} />
      ),
    },
    {
      id: 'workflow',
      header: t('flows.allRuns.columnWorkflow'),
      className: 'w-full max-w-0',
      cell: row => (
        <div className="min-w-0">
          <p className="truncate font-medium text-content" title={row.name}>
            {row.name}
          </p>
          {row.run.error && (
            <p
              className="truncate text-xs text-coral-600 dark:text-coral-300"
              title={row.run.error}>
              {row.run.error}
            </p>
          )}
        </div>
      ),
    },
    {
      id: 'started',
      header: t('flows.allRuns.columnStarted'),
      className: 'w-px whitespace-nowrap tabular-nums',
      cell: row => <DateTimeCell value={row.run.started_at} />,
    },
    {
      id: 'duration',
      header: t('flows.allRuns.columnDuration'),
      align: 'right',
      className: 'w-px whitespace-nowrap tabular-nums text-content-muted',
      cell: row => formatDuration(row.run.started_at, row.run.finished_at),
    },
  ];

  return (
    <div className="h-full p-4" data-testid="workflow-runs-page">
      <SettingsTabbedPage
        fullWidth
        title={t('flows.allRuns.title')}
        description={t('flows.allRuns.description')}
        scrollable={false}>
        {/* Bounded flex column: the runs table fills it and only its rows
            scroll, never the page. */}
        <div className="flex h-full min-h-0 flex-col">
          <DataTable<RunRow>
            title={t('flows.allRuns.tableTitle')}
            description={t('flows.allRuns.tableDesc')}
            columns={columns}
            rows={filteredRows}
            rowKey={row => row.run.id}
            renderRow={row => (
              <TableRow
                key={row.run.id}
                data-testid={`workflow-run-${row.run.id}`}
                role="link"
                tabIndex={0}
                onClick={() => openRun(row)}
                onKeyDown={event => {
                  if (event.key === 'Enter' || event.key === ' ') {
                    event.preventDefault();
                    openRun(row);
                  }
                }}
                className="cursor-pointer focus-visible:bg-surface-hover focus-visible:outline-hidden">
                {columns.map(column => (
                  <TableCell
                    key={column.id}
                    className={cn(column.align === 'right' && 'text-right', column.className)}>
                    {column.cell?.(row)}
                  </TableCell>
                ))}
              </TableRow>
            )}
            search={{
              value: query,
              onChange: setQuery,
              placeholder: t('flows.allRuns.searchPlaceholder'),
              testId: 'workflow-runs-search',
            }}
            filters={[
              {
                id: 'status',
                label: t('flows.allRuns.columnStatus'),
                options: statusOptions.map(status => ({
                  value: status,
                  label: statusLabel(status),
                })),
                selected: selectedStatus,
                onChange: setSelectedStatus,
                testId: 'workflow-runs-status-filter',
              },
            ]}
            pagination={{ pageSize: 25, testId: 'workflow-runs-pagination' }}
            loading={pageLoading}
            loadingLabel={t('flows.allRuns.loading')}
            error={pageError ? <ErrorBanner message={pageError} /> : undefined}
            empty={
              pageError ? undefined : rows.length === 0 ? (
                <p className="text-sm text-content-muted" data-testid="workflow-runs-empty">
                  {t('flows.allRuns.empty')}
                </p>
              ) : undefined
            }
            ariaLabel={t('flows.allRuns.title')}
            testId={filteredRows.length > 0 ? 'workflow-runs-list' : undefined}
          />
        </div>
      </SettingsTabbedPage>
    </div>
  );
}
