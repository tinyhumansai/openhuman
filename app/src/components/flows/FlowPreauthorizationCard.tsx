import React from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { ApprovalManifestEntry } from '../../services/api/flowsApi';
import Button from '../ui/Button';

/**
 * The consolidated save+enable pre-authorization card: one list of every
 * permission a flow run will need, with exactly two actions — "Approve all"
 * and "Deny". Presentational only; `useFlowPreauthorization` owns the RPCs.
 *
 * Row kinds:
 * - `approvable` — a permission "Approve all" grants (flow-scoped trust).
 * - `blocked` — refused by the autonomy tier; informational, not approvable.
 * - `dynamic` / `agent` — best-effort disclosures ("may still ask") so the
 *   card never over-promises a zero-prompt run.
 */
interface Props {
  entries: ApprovalManifestEntry[];
  busy: boolean;
  errorMsg?: string | null;
  onApproveAll: () => void;
  onDeny: () => void;
}

export const FlowPreauthorizationCard: React.FC<Props> = ({
  entries,
  busy,
  errorMsg,
  onApproveAll,
  onDeny,
}) => {
  const { t } = useT();

  const hasApprovable = entries.some(entry => entry.kind === 'approvable');

  const hintFor = (kind: ApprovalManifestEntry['kind']): string | null => {
    switch (kind) {
      case 'blocked':
        return t('flows.enableApproval.blockedHint');
      case 'dynamic':
        return t('flows.enableApproval.dynamicHint');
      case 'agent':
        return t('flows.enableApproval.agentHint');
      default:
        return null;
    }
  };

  return (
    <div
      role="alertdialog"
      aria-label={t('flows.enableApproval.title')}
      data-testid="flow-preauthorization-card"
      className="rounded-xl border border-ocean-300 bg-surface p-3 text-sm shadow-md dark:border-ocean-700">
      <div className="flex items-start gap-2">
        <span aria-hidden className="text-base leading-none text-ocean-700 dark:text-ocean-200">
          🔐
        </span>
        <div className="min-w-0 flex-1">
          <p className="font-semibold text-ocean-900 dark:text-ocean-100">
            {t('flows.enableApproval.title')}
          </p>
          <p className="mt-1 break-words text-ocean-800/90 dark:text-ocean-200/90">
            {t('flows.enableApproval.intro')}
          </p>

          <ul className="mt-2 max-h-56 space-y-1 overflow-y-auto">
            {entries.map(entry => {
              const hint = hintFor(entry.kind);
              const informational = entry.kind !== 'approvable';
              return (
                <li
                  key={entry.node_id}
                  data-testid={`flow-preauth-row-${entry.kind}`}
                  className={`flex items-start gap-2 rounded-lg border px-2.5 py-1.5 ${
                    informational
                      ? 'border-amber-300/60 bg-amber-50/50 dark:border-amber-700/50 dark:bg-amber-900/10'
                      : 'border-ocean-200 dark:border-ocean-800'
                  }`}>
                  <span aria-hidden className="mt-0.5 text-xs leading-none">
                    {entry.kind === 'approvable' ? '✅' : entry.kind === 'blocked' ? '⛔' : '⚠️'}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block break-words text-ink dark:text-content">
                      {entry.label}
                    </span>
                    {hint && <span className="block text-xs text-content-secondary">{hint}</span>}
                  </span>
                </li>
              );
            })}
          </ul>

          {errorMsg && <p className="mt-2 text-xs text-coral">⚠ {errorMsg}</p>}

          <div className="mt-3 flex flex-wrap items-center gap-2">
            <Button
              variant="primary"
              size="sm"
              analyticsId="flow-preauth-approve-all"
              onClick={onApproveAll}
              disabled={busy}>
              {busy
                ? t('flows.enableApproval.granting')
                : hasApprovable
                  ? t('flows.enableApproval.approveAll')
                  : t('flows.enableApproval.enableAnyway')}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              analyticsId="flow-preauth-deny"
              onClick={onDeny}
              disabled={busy}>
              {t('flows.enableApproval.deny')}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
};

/**
 * Full-screen overlay wrapper for page contexts (Flows list, canvas). Chat
 * surfaces render {@link FlowPreauthorizationCard} inline instead. No
 * backdrop dismissal on purpose: the decision is explicit — Approve all or
 * Deny — so a stray click can't leave the enable half-done.
 */
export const FlowPreauthorizationOverlay: React.FC<Props> = props => (
  <div
    className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
    data-testid="flow-preauthorization-overlay">
    <div className="w-full max-w-md">
      <FlowPreauthorizationCard {...props} />
    </div>
  </div>
);

export default FlowPreauthorizationCard;
