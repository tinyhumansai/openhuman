import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  type ApprovalAuditEntry,
  type ApprovalDecision,
  fetchRecentApprovalDecisions,
} from '../../../services/api/approvalApi';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const log = debug('ui:approval-history');

/** Render a decided timestamp as a locale string; fall back to the raw value. */
const formatDateTime = (value: string): string => {
  const ts = Date.parse(value);
  return Number.isNaN(ts) ? value : new Date(ts).toLocaleString();
};

/** Tailwind tone + i18n label key per decision variant. */
const DECISION_TONE: Record<ApprovalDecision, string> = {
  approve_once: 'bg-sage-50 text-sage ring-sage-200',
  approve_always_for_tool: 'bg-sage-50 text-sage ring-sage-200',
  deny: 'bg-coral-50 text-coral ring-coral-200',
};

const DECISION_LABEL_KEY: Record<ApprovalDecision, string> = {
  approve_once: 'settings.approvalHistory.decision.approveOnce',
  approve_always_for_tool: 'settings.approvalHistory.decision.approveAlways',
  deny: 'settings.approvalHistory.decision.deny',
};

const ApprovalHistoryPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  const [entries, setEntries] = useState<ApprovalAuditEntry[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

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

  return (
    <div>
      <SettingsHeader
        title={t('settings.approvalHistory.title')}
        showBackButton
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4" data-testid="approval-history-panel">
        <div className="flex items-center justify-between">
          <p className="text-xs text-ink-soft">{t('settings.approvalHistory.subtitle')}</p>
          <button
            type="button"
            onClick={handleRefresh}
            disabled={isLoading}
            data-testid="approval-history-refresh"
            className="rounded bg-primary-500 px-3 py-1 text-xs text-white hover:bg-primary-600 disabled:opacity-50">
            {t('settings.approvalHistory.refresh')}
          </button>
        </div>

        {isLoading ? (
          <p className="text-sm text-ink-soft" data-testid="approval-history-loading">
            {t('settings.approvalHistory.loading')}
          </p>
        ) : error ? (
          <div className="space-y-2" data-testid="approval-history-error">
            <p className="text-sm text-coral">{error}</p>
            <button
              type="button"
              onClick={handleRefresh}
              className="text-xs text-primary-600 hover:underline">
              {t('settings.approvalHistory.retry')}
            </button>
          </div>
        ) : entries.length === 0 ? (
          <p className="text-sm text-ink-soft" data-testid="approval-history-empty">
            {t('settings.approvalHistory.emptyState')}
          </p>
        ) : (
          <ul className="space-y-2" data-testid="approval-history-list">
            {entries.map(entry => (
              <li
                key={entry.request_id}
                className="rounded-lg border border-line p-3 space-y-1"
                data-testid="approval-history-row">
                <div className="flex items-center justify-between gap-2">
                  <span className="font-mono text-xs text-ink truncate">{entry.tool_name}</span>
                  <span
                    className={`inline-flex shrink-0 items-center rounded-full px-2 py-0.5 text-xs font-medium ring-1 ${DECISION_TONE[entry.decision]}`}
                    data-testid={`approval-history-decision-${entry.decision}`}>
                    {t(DECISION_LABEL_KEY[entry.decision])}
                  </span>
                </div>
                <p className="text-xs text-ink-soft">{entry.action_summary}</p>
                <p className="text-[11px] text-ink-soft">
                  {t('settings.approvalHistory.decidedAt').replace(
                    '{date}',
                    formatDateTime(entry.decided_at)
                  )}
                </p>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
};

export default ApprovalHistoryPanel;
