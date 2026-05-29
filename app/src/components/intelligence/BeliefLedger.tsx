/**
 * Belief Ledger — presentational view of how the agent's beliefs about the
 * user changed over time. Pure: it derives everything from `relations` via the
 * deterministic engine and renders per-claim revision timelines. All data
 * fetching, explanation, and correction live in the container (BeliefLedgerTab).
 */
import { useMemo, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type BeliefHistory,
  buildBeliefHistories,
  type RevisionKind,
} from '../../lib/memory/beliefLedger';
import type { GraphRelation } from '../../utils/tauriCommands/memory';

interface BeliefLedgerProps {
  relations: GraphRelation[];
  loading?: boolean;
  /** Injected reference time (epoch seconds) for deterministic scoring; defaults to now. */
  nowSeconds?: number;
  /** Map of claimKey -> explanation text, rendered inline under the matching card. */
  explanationByKey?: Record<string, string>;
  /** claimKey currently awaiting an explanation (shows a "explaining…" state). */
  explainingKey?: string | null;
  onExplain?: (history: BeliefHistory) => void;
  onCorrect?: (history: BeliefHistory) => void;
}

// Confidence bands for the pill colour: >= HIGH is trustworthy (sage),
// >= MEDIUM is contested-but-leaning (amber), below is a near-tie (coral).
const CONFIDENCE_HIGH = 0.66;
const CONFIDENCE_MEDIUM = 0.33;

function confidenceTone(confidence: number): string {
  if (confidence >= CONFIDENCE_HIGH) return 'text-sage-700 dark:text-sage-300';
  if (confidence >= CONFIDENCE_MEDIUM) return 'text-amber-700 dark:text-amber-300';
  return 'text-coral-700 dark:text-coral-300';
}

function revisionLabelKey(kind: RevisionKind): string {
  return `beliefLedger.revision.${kind}`;
}

function formatDate(epochSeconds: number): string {
  return new Date(epochSeconds * 1000).toISOString().slice(0, 10);
}

const BeliefLedger = ({
  relations,
  loading,
  nowSeconds,
  explanationByKey,
  explainingKey,
  onExplain,
  onCorrect,
}: BeliefLedgerProps) => {
  const { t } = useT();
  // Capture "now" once on mount via a lazy initializer (Date.now in a
  // callback, never during render — satisfies react-hooks/purity) so the
  // memoized scoring is stable across re-renders. Tests inject `nowSeconds`
  // for determinism.
  const [fallbackNow] = useState(() => Math.floor(Date.now() / 1000));
  const now = nowSeconds ?? fallbackNow;
  const histories = useMemo(() => buildBeliefHistories(relations, now), [relations, now]);

  if (loading) {
    return (
      <div
        className="space-y-3"
        role="status"
        aria-label={t('beliefLedger.loading')}
        data-testid="belief-ledger-loading">
        {[0, 1, 2].map(i => (
          <div
            key={i}
            className="animate-pulse rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 h-24"
          />
        ))}
      </div>
    );
  }

  if (histories.length === 0) {
    return (
      <div className="py-10 text-center">
        <h3 className="text-sm font-semibold text-stone-700 dark:text-neutral-200">
          {t('beliefLedger.empty')}
        </h3>
        <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
          {t('beliefLedger.emptyHint')}
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {histories.map(history => (
        <article
          key={history.claimKey}
          aria-label={`${history.subject} ${history.predicate}`}
          className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-5">
          {/* Header */}
          <div className="flex items-start justify-between gap-3 flex-wrap">
            <div className="min-w-0">
              <div className="flex items-center gap-2 flex-wrap">
                <span className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                  {history.subject}
                </span>
                {history.current.subjectType && (
                  <span className="inline-flex items-center rounded px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wider bg-stone-100 dark:bg-neutral-800 text-stone-500 dark:text-neutral-400">
                    {history.current.subjectType}
                  </span>
                )}
                <span className="text-xs italic text-stone-500 dark:text-neutral-400">
                  {history.predicate}
                </span>
              </div>
              <p className="mt-1 text-base font-medium text-stone-900 dark:text-neutral-100 break-words">
                {history.current.object}
                <span className="ml-2 text-[10px] font-normal uppercase tracking-wider text-stone-400 dark:text-neutral-500">
                  {t('beliefLedger.currentBelief')}
                </span>
              </p>
            </div>
            <div className="flex items-center gap-1.5 shrink-0">
              {history.hasConflict && (
                <span className="inline-flex items-center rounded px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wider bg-coral-100 dark:bg-coral-500/20 text-coral-700 dark:text-coral-300">
                  {t('beliefLedger.conflictBadge')}
                </span>
              )}
              <span
                role="img"
                aria-label={`${t('beliefLedger.confidence')} ${Math.round(history.confidence * 100)}%`}
                className={`inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-semibold tabular-nums ${confidenceTone(
                  history.confidence
                )}`}>
                {Math.round(history.confidence * 100)}%
              </span>
              <span
                role="img"
                aria-label={`${t('beliefLedger.evidence')} ${history.current.evidenceCount}`}
                className="inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-medium tabular-nums bg-stone-100 dark:bg-neutral-800 text-stone-500 dark:text-neutral-400">
                x{history.current.evidenceCount}
              </span>
            </div>
          </div>

          {/* Revision timeline */}
          <div className="mt-3">
            <h4 className="text-[10px] font-semibold uppercase tracking-wider text-stone-400 dark:text-neutral-500 mb-1.5">
              {t('beliefLedger.historyHeading')}
              {history.distinctValueCount > 1 && (
                <span className="ml-1 normal-case font-normal">
                  (
                  {t('beliefLedger.distinctValues').replace(
                    '{n}',
                    String(history.distinctValueCount)
                  )}
                  )
                </span>
              )}
            </h4>
            <ol className="border-l border-stone-200 dark:border-neutral-700 pl-3 space-y-2">
              {history.revisions.map((rev, idx) => (
                <li
                  key={`${rev.version.normalizedObject}-${rev.version.updatedAt}-${idx}`}
                  className="relative">
                  <span
                    aria-hidden="true"
                    className={`absolute -left-[17px] top-1.5 h-2 w-2 rounded-full ${
                      rev.isCurrent
                        ? 'bg-primary-500 ring-2 ring-primary-200 dark:ring-primary-500/30'
                        : 'bg-stone-300 dark:bg-neutral-600'
                    }`}
                  />
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="text-xs font-medium text-stone-800 dark:text-neutral-200 break-words">
                      {rev.version.object}
                    </span>
                    <span className="text-[9px] font-semibold uppercase tracking-wider text-stone-400 dark:text-neutral-500">
                      {t(revisionLabelKey(rev.kind))}
                    </span>
                    <span className="text-[10px] text-stone-400 dark:text-neutral-500 tabular-nums">
                      {formatDate(rev.version.updatedAt)} · x{rev.version.evidenceCount}
                    </span>
                  </div>
                </li>
              ))}
            </ol>
          </div>

          {/* Actions */}
          <div className="mt-3 flex items-center gap-2">
            {history.hasConflict && onExplain && (
              <button
                type="button"
                onClick={() => onExplain(history)}
                disabled={explainingKey === history.claimKey}
                aria-label={`${t('beliefLedger.explain')} — ${history.subject} ${history.predicate}`}
                className="rounded-lg border border-stone-200 dark:border-neutral-700 px-2.5 py-1 text-xs font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-100 dark:hover:bg-neutral-800 disabled:opacity-50 disabled:cursor-not-allowed">
                {explainingKey === history.claimKey
                  ? t('beliefLedger.explaining')
                  : t('beliefLedger.explain')}
              </button>
            )}
            {onCorrect && (
              <button
                type="button"
                onClick={() => onCorrect(history)}
                aria-label={`${t('beliefLedger.setTruth')} — ${history.subject} ${history.predicate}`}
                className="rounded-lg border border-stone-200 dark:border-neutral-700 px-2.5 py-1 text-xs font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-100 dark:hover:bg-neutral-800">
                {t('beliefLedger.setTruth')}
              </button>
            )}
          </div>

          {/* Inline explanation */}
          {explanationByKey?.[history.claimKey] && (
            <div className="mt-3 rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 p-3">
              <h5 className="text-[10px] font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400 mb-1">
                {t('beliefLedger.explanationHeading')}
              </h5>
              <p className="text-xs text-stone-700 dark:text-neutral-200 whitespace-pre-wrap break-words">
                {explanationByKey[history.claimKey]}
              </p>
            </div>
          )}
        </article>
      ))}
    </div>
  );
};

export default BeliefLedger;
