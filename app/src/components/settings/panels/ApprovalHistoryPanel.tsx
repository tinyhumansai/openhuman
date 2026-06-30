import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  type ApprovalAuditEntry,
  type ApprovalDecision,
  fetchRecentApprovalDecisions,
} from '../../../services/api/approvalApi';
import Button from '../../ui/Button';
import {
  SettingsBadge,
  SettingsEmptyState,
  SettingsSection,
  SettingsStatusLine,
} from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

const log = debug('ui:approval-history');

/** Render a decided timestamp as a locale string; fall back to the raw value. */
const formatDateTime = (value: string): string => {
  const ts = Date.parse(value);
  return Number.isNaN(ts) ? value : new Date(ts).toLocaleString();
};

/** SettingsBadge variant per decision variant. */
const DECISION_BADGE_VARIANT: Record<
  ApprovalDecision,
  'success' | 'danger' | 'warning' | 'neutral' | 'primary'
> = { approve_once: 'success', approve_always_for_tool: 'success', deny: 'danger' };

const DECISION_LABEL_KEY: Record<ApprovalDecision, string> = {
  approve_once: 'settings.approvalHistory.decision.approveOnce',
  approve_always_for_tool: 'settings.approvalHistory.decision.approveAlways',
  deny: 'settings.approvalHistory.decision.deny',
};

const ApprovalHistoryPanel = () => {
  const { t } = useT();

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
    <SettingsPanel testId="approval-history-panel">
      <SettingsSection>
        <div className="px-4 py-3 flex items-center justify-between gap-2">
          <p className="text-xs text-content-muted">{t('settings.approvalHistory.subtitle')}</p>
          <Button
            type="button"
            variant="primary"
            size="xs"
            onClick={handleRefresh}
            disabled={isLoading}
            data-testid="approval-history-refresh">
            {t('settings.approvalHistory.refresh')}
          </Button>
        </div>

        {isLoading ? (
          <div
            className="px-4 py-4 text-sm text-content-muted"
            data-testid="approval-history-loading">
            {t('settings.approvalHistory.loading')}
          </div>
        ) : error ? (
          <div className="px-4 py-4 space-y-2" data-testid="approval-history-error">
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
        ) : entries.length === 0 ? (
          <div className="px-4 py-8 text-center" data-testid="approval-history-empty">
            <SettingsEmptyState label={t('settings.approvalHistory.emptyState')} />
          </div>
        ) : (
          <ul
            className="divide-y divide-line-subtle dark:divide-neutral-800"
            data-testid="approval-history-list">
            {entries.map(entry => (
              <li
                key={entry.request_id}
                className="px-4 py-3 space-y-1"
                data-testid="approval-history-row">
                <div className="flex items-center justify-between gap-2">
                  <span className="font-mono text-xs text-content truncate">{entry.tool_name}</span>
                  <span
                    data-testid={`approval-history-decision-${entry.decision}`}
                    className="flex-shrink-0">
                    <SettingsBadge variant={DECISION_BADGE_VARIANT[entry.decision]}>
                      {t(DECISION_LABEL_KEY[entry.decision])}
                    </SettingsBadge>
                  </span>
                </div>
                <p className="text-xs text-content-muted">{entry.action_summary}</p>
                <p className="text-[11px] text-content-muted">
                  {t('settings.approvalHistory.decidedAt').replace(
                    '{date}',
                    formatDateTime(entry.decided_at)
                  )}
                </p>
              </li>
            ))}
          </ul>
        )}
      </SettingsSection>
    </SettingsPanel>
  );
};

export default ApprovalHistoryPanel;
