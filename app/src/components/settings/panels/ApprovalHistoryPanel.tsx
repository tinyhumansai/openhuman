import debug from 'debug';
import { RefreshCw } from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  type ApprovalAuditEntry,
  type ApprovalDecision,
  fetchRecentApprovalDecisions,
} from '../../../services/api/approvalApi';
import Badge from '../../ui/Badge';
import Button from '../../ui/Button';
import DataTable, { type DataTableColumn } from '../../ui/DataTable';
import EmptyState from '../../ui/EmptyState';
import { TableCell, TableRow } from '../../ui/Table';
import { SettingsStatusLine } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

const log = debug('ui:approval-history');

/**
 * Render a decided timestamp as two lines — date over time — in the user's
 * locale; an unparseable value is shown raw on one line.
 */
const DateTimeCell = ({ value }: { value: string }) => {
  const ts = Date.parse(value);
  if (Number.isNaN(ts)) return <>{value}</>;
  const date = new Date(ts);
  return (
    <span className="flex flex-col leading-tight" title={date.toLocaleString()}>
      <span className="text-content">{date.toLocaleDateString()}</span>
      <span className="text-xs text-content-muted">{date.toLocaleTimeString()}</span>
    </span>
  );
};

const pct = (millis: number | null | undefined) =>
  millis == null ? '–' : `${(millis / 10).toFixed(millis % 10 === 0 ? 0 : 1)}%`;

/**
 * The automatic-decision line under a verdict: who decided, the Jev scores and
 * when the grant expires. Renders nothing for a human decision.
 */
const AutoVerdict = ({ entry }: { entry: ApprovalAuditEntry }) => {
  const { t } = useT();
  if (!entry.decided_by?.startsWith('auto:')) return null;
  const jev = entry.decided_by === 'auto:jev';
  const judge = entry.judge;
  return (
    <div className="mt-1 flex flex-col items-end gap-0.5" data-testid="approval-history-auto">
      <Badge variant="primary" data-testid="approval-history-auto-badge">
        {t(jev ? 'settings.approvalHistory.autoJev' : 'settings.approvalHistory.autoPolicy')}
      </Badge>
      {jev && judge && (
        <span className="text-xs text-content-muted" data-testid="approval-history-judge-scores">
          {t('settings.approvalHistory.judgeScores')
            .replace('{intent}', pct(judge.intent_millis))
            .replace('{reversible}', pct(judge.reversible_millis))
            .replace('{exfiltrate}', pct(judge.exfiltrate_millis))}
          {judge.cached ? ` · ${t('settings.approvalHistory.judgeCached')}` : ''}
        </span>
      )}
      {entry.expires_at && (
        <span className="text-xs text-content-muted" data-testid="approval-history-expiry">
          {t('settings.approvalHistory.judgeExpires').replace(
            '{time}',
            new Date(entry.expires_at).toLocaleString()
          )}
        </span>
      )}
    </div>
  );
};

/** Badge tone per decision: approvals read sage, a denial coral. */
const DECISION_VARIANT: Record<ApprovalDecision, 'success' | 'danger'> = {
  approve_once: 'success',
  approve_always_for_tool: 'success',
  approve_always_for_flow: 'success',
  deny: 'danger',
};

const DECISION_LABEL_KEY: Record<ApprovalDecision, string> = {
  approve_once: 'settings.approvalHistory.decision.approveOnce',
  approve_always_for_tool: 'settings.approvalHistory.decision.approveAlways',
  approve_always_for_flow: 'settings.approvalHistory.decision.approveAlwaysFlow',
  deny: 'settings.approvalHistory.decision.deny',
};

const DECISION_ORDER: ApprovalDecision[] = [
  'approve_once',
  'approve_always_for_tool',
  'approve_always_for_flow',
  'deny',
];

const ApprovalHistoryPanel = () => {
  const { t } = useT();

  const [entries, setEntries] = useState<ApprovalAuditEntry[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [selectedDecisions, setSelectedDecisions] = useState<ReadonlySet<string>>(new Set());

  // Monotonic guard so an out-of-order (slower) response can't clobber a
  // fresher one when the user taps Refresh rapidly (last request wins).
  const loadSeqRef = useRef(0);

  // Runs the fetch and only ever calls setState AFTER the await, so it is safe
  // to invoke straight from the mount effect without tripping
  // react-hooks/set-state-in-effect. The synchronous spinner reset lives in the
  // Refresh event handler below, where synchronous setState is expected.
  const runLoad = useCallback(
    async (seq: number) => {
      log('load start %o', { seq });
      try {
        const rows = await fetchRecentApprovalDecisions();
        if (seq !== loadSeqRef.current) {
          log('stale response discarded %o', { seq, latest: loadSeqRef.current });
          return;
        }
        setEntries(rows);
        setError(null);
        log('load ok %o', { seq, count: rows.length });
      } catch (e) {
        if (seq !== loadSeqRef.current) return;
        // Never leak raw backend error text into the UI; localized fallback only.
        log('load failed %o', e);
        setError(t('settings.approvalHistory.errorGeneric'));
      } finally {
        if (seq === loadSeqRef.current) setIsLoading(false);
      }
    },
    [t]
  );

  useEffect(() => {
    void runLoad(++loadSeqRef.current);
  }, [runLoad]);

  const handleRefresh = () => {
    setIsLoading(true);
    setError(null);
    void runLoad(++loadSeqRef.current);
  };

  const filteredEntries = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return entries.filter(entry => {
      if (selectedDecisions.size > 0 && !selectedDecisions.has(entry.decision)) return false;
      if (!needle) return true;
      return (
        (entry.tool_name ?? '').toLowerCase().includes(needle) ||
        (entry.action_summary ?? '').toLowerCase().includes(needle)
      );
    });
  }, [entries, query, selectedDecisions]);

  const columns: DataTableColumn<ApprovalAuditEntry>[] = [
    {
      id: 'time',
      header: t('settings.approvalHistory.column.time'),
      // `w-px` + nowrap: fixed-content columns shrink to fit, so the
      // tool/action column takes all the remaining width.
      className: 'w-px whitespace-nowrap tabular-nums',
      cell: entry => <DateTimeCell value={entry.decided_at} />,
    },
    {
      id: 'tool',
      header: t('settings.approvalHistory.column.tool'),
      // `max-w-0 w-full` is what lets a table cell truncate instead of growing.
      className: 'w-full max-w-0',
      cell: entry => (
        <div className="min-w-0 space-y-0.5">
          <p className="truncate font-mono text-xs text-content" title={entry.tool_name}>
            {entry.tool_name}
          </p>
          <p className="truncate text-xs text-content-muted" title={entry.action_summary}>
            {entry.action_summary}
          </p>
        </div>
      ),
    },
    {
      id: 'decision',
      header: t('settings.approvalHistory.column.decision'),
      align: 'right',
      className: 'w-px whitespace-nowrap',
      cell: entry => (
        <div className="flex flex-col items-end">
          <Badge
            variant={DECISION_VARIANT[entry.decision]}
            data-testid={`approval-history-decision-${entry.decision}`}>
            {t(DECISION_LABEL_KEY[entry.decision])}
          </Badge>
          <AutoVerdict entry={entry} />
        </div>
      ),
    },
  ];

  const hasEntries = filteredEntries.length > 0;

  return (
    // Non-scrolling page body: the table card fills it and only its rows scroll.
    <SettingsPanel
      testId="approval-history-panel"
      description={t('settings.approvalHistory.subtitle')}
      scrollable={false}
      bodyClassName="flex h-full min-h-0 flex-col gap-4">
      <DataTable<ApprovalAuditEntry>
        title={t('settings.approvalHistory.tableTitle')}
        description={t('settings.approvalHistory.tableDesc')}
        pagination={{ pageSize: 25, testId: 'approval-history-pagination' }}
        columns={columns}
        rows={filteredEntries}
        rowKey={entry => entry.request_id}
        renderRow={entry => (
          <TableRow key={entry.request_id} data-testid="approval-history-row">
            {columns.map(column => (
              <TableCell
                key={column.id}
                className={
                  column.align === 'right'
                    ? `${column.className ?? ''} text-right`
                    : column.className
                }>
                {column.cell?.(entry)}
              </TableCell>
            ))}
          </TableRow>
        )}
        actions={
          <Button
            type="button"
            variant="secondary"
            size="sm"
            leadingIcon={<RefreshCw className="h-3.5 w-3.5" aria-hidden />}
            onClick={handleRefresh}
            disabled={isLoading}
            data-testid="approval-history-refresh">
            {t('settings.approvalHistory.refresh')}
          </Button>
        }
        search={{
          value: query,
          onChange: setQuery,
          placeholder: t('settings.approvalHistory.searchPlaceholder'),
          testId: 'approval-history-search',
        }}
        filters={[
          {
            id: 'decision',
            label: t('settings.approvalHistory.filterDecision'),
            ariaLabel: t('settings.approvalHistory.filterDecisionAriaLabel'),
            options: DECISION_ORDER.map(decision => ({
              value: decision,
              label: t(DECISION_LABEL_KEY[decision]),
            })),
            selected: selectedDecisions,
            onChange: setSelectedDecisions,
            testId: 'approval-history-decision-filter',
          },
        ]}
        loading={isLoading}
        loadingTestId="approval-history-loading"
        loadingLabel={t('settings.approvalHistory.loading')}
        error={
          error ? (
            <div className="space-y-2" data-testid="approval-history-error">
              <SettingsStatusLine saving={false} error={error} savingLabel="" />
              <Button
                type="button"
                variant="tertiary"
                size="xs"
                onClick={handleRefresh}
                className="text-primary-600 dark:text-primary-400">
                {t('settings.approvalHistory.retry')}
              </Button>
            </div>
          ) : undefined
        }
        empty={
          error ? undefined : (
            <div data-testid="approval-history-empty">
              <EmptyState label={t('settings.approvalHistory.emptyState')} />
            </div>
          )
        }
        ariaLabel={t('settings.approvalHistory.tableAriaLabel')}
        testId={hasEntries ? 'approval-history-list' : undefined}
      />
    </SettingsPanel>
  );
};

export default ApprovalHistoryPanel;
