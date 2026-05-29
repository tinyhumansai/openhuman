/**
 * Prediction Calibration Ledger — presentational view. Pure: it derives the
 * calibration report from `records` via the deterministic engine and renders
 * the metrics, curve, decomposition, trend, and prediction lists. All I/O
 * (load/save) and id/timestamp minting live in the container.
 */
import { useMemo, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  buildCalibrationReport,
  isOverdue,
  isResolved,
  type PredictionOutcome,
  type PredictionRecord,
} from '../../lib/memory/predictionLedger';

/** Raw add-form values; the container converts % -> fraction and mints id/time. */
export interface AddPredictionFormValues {
  statement: string;
  confidencePercent: number;
  resolveBy: number | null;
  category: string | null;
}

interface PredictionLedgerProps {
  records: PredictionRecord[];
  loading?: boolean;
  /** Injected reference time (epoch seconds) for overdue marking; defaults to now. */
  nowSeconds?: number;
  onAdd?: (values: AddPredictionFormValues) => void;
  onResolve?: (id: string, outcome: PredictionOutcome) => void;
  onDelete?: (id: string) => void;
}

const WELL_CALIBRATED_BAND = 0.05;
const DEFAULT_CONFIDENCE_PERCENT = 70;
const pct = (fraction: number): number => Math.round(fraction * 100);
const clampPercent = (n: number): number =>
  Number.isFinite(n) ? Math.min(100, Math.max(0, n)) : 0;

const PredictionLedger = ({
  records,
  loading,
  nowSeconds,
  onAdd,
  onResolve,
  onDelete,
}: PredictionLedgerProps) => {
  const { t } = useT();
  const [fallbackNow] = useState(() => Math.floor(Date.now() / 1000));
  const now = nowSeconds ?? fallbackNow;
  const report = useMemo(() => buildCalibrationReport(records), [records]);

  // Add-form state.
  const [statement, setStatement] = useState('');
  const [confidence, setConfidence] = useState(DEFAULT_CONFIDENCE_PERCENT);
  const [resolveBy, setResolveBy] = useState('');
  const [category, setCategory] = useState('');

  const submitAdd = () => {
    if (!onAdd || statement.trim().length === 0) return;
    // Parse the date-input value as LOCAL midnight (appending the time avoids
    // the YYYY-MM-DD => UTC-midnight interpretation, which skews "overdue").
    const resolveByMs = resolveBy ? new Date(`${resolveBy}T00:00:00`).getTime() : NaN;
    onAdd({
      statement: statement.trim(),
      confidencePercent: clampPercent(confidence),
      resolveBy: Number.isFinite(resolveByMs) ? Math.floor(resolveByMs / 1000) : null,
      category: category.trim() || null,
    });
    setStatement('');
    setConfidence(DEFAULT_CONFIDENCE_PERCENT);
    setResolveBy('');
    setCategory('');
  };

  if (loading) {
    return (
      <div
        className="space-y-3"
        role="status"
        aria-label={t('predictionLedger.loading')}
        data-testid="prediction-ledger-loading">
        {[0, 1, 2].map(i => (
          <div
            key={i}
            className="animate-pulse rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 h-20"
          />
        ))}
      </div>
    );
  }

  const open = records.filter(r => !isResolved(r));
  const resolved = records.filter(isResolved);

  const confidenceTone =
    Math.abs(report.overconfidence) < WELL_CALIBRATED_BAND
      ? 'bg-sage-100 dark:bg-sage-500/20 text-sage-700 dark:text-sage-300'
      : 'bg-amber-100 dark:bg-amber-500/20 text-amber-700 dark:text-amber-300';
  const confidenceLabel =
    Math.abs(report.overconfidence) < WELL_CALIBRATED_BAND
      ? t('predictionLedger.wellCalibratedLabel')
      : report.overconfidence > 0
        ? t('predictionLedger.overconfidentLabel')
        : t('predictionLedger.underconfidentLabel');

  return (
    <div className="space-y-4">
      <div
        role="note"
        className="rounded-lg border border-primary-200 dark:border-primary-500/30 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-xs text-stone-700 dark:text-neutral-200">
        <p className="font-medium mb-1">{t('predictionLedger.title')}</p>
        <p>{t('predictionLedger.intro')}</p>
      </div>

      {/* Add form */}
      <form
        className="rounded-xl border border-stone-200 dark:border-neutral-800 p-4 space-y-2"
        onSubmit={e => {
          e.preventDefault();
          submitAdd();
        }}>
        <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
          {t('predictionLedger.addHeading')}
        </h3>
        <input
          type="text"
          value={statement}
          onChange={e => setStatement(e.target.value)}
          placeholder={t('predictionLedger.statementPlaceholder')}
          aria-label={t('predictionLedger.statementLabel')}
          className="w-full rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-1.5 text-sm text-stone-800 dark:text-neutral-100"
        />
        <div className="flex flex-wrap items-center gap-3">
          <label className="flex items-center gap-2 text-xs text-stone-600 dark:text-neutral-300">
            {t('predictionLedger.confidenceLabel')}
            <input
              type="number"
              min={0}
              max={100}
              value={confidence}
              onChange={e => setConfidence(clampPercent(Number(e.target.value)))}
              className="w-20 rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1 text-sm tabular-nums text-stone-800 dark:text-neutral-100"
            />
          </label>
          <label className="flex items-center gap-2 text-xs text-stone-600 dark:text-neutral-300">
            {t('predictionLedger.resolveByLabel')}
            <input
              type="date"
              value={resolveBy}
              onChange={e => setResolveBy(e.target.value)}
              className="rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1 text-sm text-stone-800 dark:text-neutral-100"
            />
          </label>
          <input
            type="text"
            value={category}
            onChange={e => setCategory(e.target.value)}
            placeholder={t('predictionLedger.categoryLabel')}
            aria-label={t('predictionLedger.categoryLabel')}
            className="flex-1 min-w-[8rem] rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1 text-sm text-stone-800 dark:text-neutral-100"
          />
          <button
            type="submit"
            disabled={statement.trim().length === 0}
            className="rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-primary-600 disabled:opacity-50 disabled:cursor-not-allowed">
            {t('predictionLedger.addButton')}
          </button>
        </div>
      </form>

      {/* Headline metrics */}
      {resolved.length > 0 && (
        <div className="grid gap-2 sm:grid-cols-3">
          <div className="rounded-lg border border-stone-200 dark:border-neutral-800 p-3">
            <div className="text-[10px] uppercase tracking-wider text-stone-400 dark:text-neutral-500">
              {t('predictionLedger.brierLabel')}
            </div>
            <div className="text-lg font-semibold tabular-nums text-stone-900 dark:text-neutral-100">
              {report.brierScore.toFixed(3)}
            </div>
            <div className="text-[10px] text-stone-400 dark:text-neutral-500">
              {t('predictionLedger.brierHint')}
            </div>
          </div>
          <div className="rounded-lg border border-stone-200 dark:border-neutral-800 p-3">
            <div className="text-[10px] uppercase tracking-wider text-stone-400 dark:text-neutral-500">
              {t('predictionLedger.meanConfidenceLabel')} · {t('predictionLedger.baseRateLabel')}
            </div>
            <div className="text-lg font-semibold tabular-nums text-stone-900 dark:text-neutral-100">
              {pct(report.meanConfidence)}% · {pct(report.baseRate)}%
            </div>
          </div>
          <div className="rounded-lg border border-stone-200 dark:border-neutral-800 p-3 flex items-center">
            <span
              className={`inline-flex items-center rounded px-2 py-1 text-xs font-semibold ${confidenceTone}`}>
              {confidenceLabel}
            </span>
          </div>
        </div>
      )}

      {/* Calibration curve */}
      {resolved.length > 0 && (
        <section aria-labelledby="prediction-curve-heading" className="space-y-1">
          <h3
            id="prediction-curve-heading"
            className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
            {t('predictionLedger.curveHeading')}
          </h3>
          <ul className="space-y-1">
            {report.bins
              .filter(bin => bin.count > 0)
              .map(bin => (
                <li
                  key={bin.index}
                  aria-label={t('predictionLedger.binAria')
                    .replace('{range}', `${pct(bin.lower)}-${pct(bin.upper)}%`)
                    .replace('{predicted}', String(pct(bin.meanPredicted)))
                    .replace('{actual}', String(pct(bin.hitRate)))
                    .replace('{n}', String(bin.count))}
                  className="flex items-center gap-2 text-[11px] tabular-nums">
                  <span className="w-14 shrink-0 text-stone-400 dark:text-neutral-500">
                    {pct(bin.lower)}-{pct(bin.upper)}%
                  </span>
                  <div className="flex-1 h-3 rounded bg-stone-100 dark:bg-neutral-800 relative overflow-hidden">
                    <div
                      className="absolute inset-y-0 left-0 bg-primary-400/60"
                      style={{ width: `${pct(bin.hitRate)}%` }}
                    />
                    <div
                      className="absolute inset-y-0 w-px bg-coral-500"
                      style={{ left: `${pct(bin.meanPredicted)}%` }}
                    />
                  </div>
                  <span className="w-24 shrink-0 text-stone-500 dark:text-neutral-400">
                    {t('predictionLedger.curvePredicted')} {pct(bin.meanPredicted)}% ·{' '}
                    {t('predictionLedger.curveActual')} {pct(bin.hitRate)}%
                  </span>
                </li>
              ))}
          </ul>
          {/* Decomposition */}
          <div className="flex flex-wrap gap-3 pt-1 text-[11px] text-stone-500 dark:text-neutral-400 tabular-nums">
            <span>
              {t('predictionLedger.reliabilityLabel')}: {report.reliability.toFixed(3)}
            </span>
            <span>
              {t('predictionLedger.resolutionLabel')}: {report.resolution.toFixed(3)}
            </span>
            <span>
              {t('predictionLedger.uncertaintyLabel')}: {report.uncertainty.toFixed(3)}
            </span>
          </div>
          <p className="text-[10px] text-stone-400 dark:text-neutral-500">
            {t('predictionLedger.decompositionHint')}
          </p>
        </section>
      )}

      {/* Cumulative-Brier trend (lower line = better calibration over time) */}
      {report.trend.length >= 2 && (
        <section aria-labelledby="prediction-trend-heading" className="space-y-1">
          <h3
            id="prediction-trend-heading"
            className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
            {t('predictionLedger.trendHeading')}
          </h3>
          <svg
            viewBox="0 0 100 24"
            preserveAspectRatio="none"
            role="img"
            aria-label={t('predictionLedger.trendCaption')
              .replace('{first}', report.trend[0].brierScore.toFixed(3))
              .replace('{last}', report.trend[report.trend.length - 1].brierScore.toFixed(3))
              .replace('{n}', String(report.trend.length))}
            className="w-full h-8 rounded bg-stone-50 dark:bg-neutral-800/60 text-primary-500">
            <polyline
              fill="none"
              stroke="currentColor"
              strokeWidth={1}
              strokeLinejoin="round"
              points={report.trend
                .map((point, i) => {
                  const x = (i / (report.trend.length - 1)) * 100;
                  // SVG y grows downward; invert so a lower (better) Brier sits
                  // lower on screen, matching the "lower is better" framing.
                  const y = 24 - Math.min(1, Math.max(0, point.brierScore)) * 24;
                  return `${x.toFixed(2)},${y.toFixed(2)}`;
                })
                .join(' ')}
            />
          </svg>
          <p className="text-[10px] text-stone-400 dark:text-neutral-500 tabular-nums">
            {t('predictionLedger.trendCaption')
              .replace('{first}', report.trend[0].brierScore.toFixed(3))
              .replace('{last}', report.trend[report.trend.length - 1].brierScore.toFixed(3))
              .replace('{n}', String(report.trend.length))}
          </p>
        </section>
      )}

      {/* Open predictions */}
      {open.length > 0 && (
        <section className="space-y-1">
          <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
            {t('predictionLedger.openHeading')}
          </h3>
          <ul className="space-y-1.5">
            {open.map(record => (
              <li
                key={record.id}
                className="flex items-center justify-between gap-2 rounded-lg border border-stone-200 dark:border-neutral-800 px-3 py-2">
                <div className="min-w-0">
                  <p className="text-sm text-stone-800 dark:text-neutral-100 break-words">
                    {record.statement}
                  </p>
                  <p className="text-[11px] text-stone-400 dark:text-neutral-500 tabular-nums">
                    {pct(record.confidence)}%
                    {isOverdue(record, now) && (
                      <span className="ml-2 inline-flex items-center rounded px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wider bg-coral-100 dark:bg-coral-500/20 text-coral-700 dark:text-coral-300">
                        {t('predictionLedger.overdueBadge')}
                      </span>
                    )}
                  </p>
                </div>
                {onResolve && (
                  <div className="flex items-center gap-1.5 shrink-0">
                    <button
                      type="button"
                      onClick={() => onResolve(record.id, 'true')}
                      aria-label={t('predictionLedger.resolveAria')
                        .replace('{outcome}', t('predictionLedger.resolveTrue'))
                        .replace('{statement}', record.statement)}
                      className="rounded-lg border border-sage-200 dark:border-sage-500/30 px-2 py-1 text-[11px] font-medium text-sage-700 dark:text-sage-300 hover:bg-sage-50 dark:hover:bg-sage-500/10">
                      {t('predictionLedger.resolveTrue')}
                    </button>
                    <button
                      type="button"
                      onClick={() => onResolve(record.id, 'false')}
                      aria-label={t('predictionLedger.resolveAria')
                        .replace('{outcome}', t('predictionLedger.resolveFalse'))
                        .replace('{statement}', record.statement)}
                      className="rounded-lg border border-coral-200 dark:border-coral-500/30 px-2 py-1 text-[11px] font-medium text-coral-700 dark:text-coral-300 hover:bg-coral-50 dark:hover:bg-coral-500/10">
                      {t('predictionLedger.resolveFalse')}
                    </button>
                  </div>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}

      {/* Resolved predictions */}
      {resolved.length > 0 && (
        <section className="space-y-1">
          <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
            {t('predictionLedger.resolvedHeading')}
          </h3>
          <ul className="space-y-1.5">
            {resolved.map(record => (
              <li
                key={record.id}
                className="flex items-center justify-between gap-2 rounded-lg border border-stone-200 dark:border-neutral-800 px-3 py-2">
                <div className="min-w-0">
                  <p className="text-sm text-stone-800 dark:text-neutral-100 break-words">
                    {record.statement}
                  </p>
                  <p className="text-[11px] text-stone-400 dark:text-neutral-500 tabular-nums">
                    {t('predictionLedger.predictedVsOutcome')
                      .replace('{predicted}', String(pct(record.confidence)))
                      .replace(
                        '{outcome}',
                        record.outcome === 'true'
                          ? t('predictionLedger.resolveTrue')
                          : t('predictionLedger.resolveFalse')
                      )}
                  </p>
                </div>
                {onDelete && (
                  <button
                    type="button"
                    onClick={() => onDelete(record.id)}
                    aria-label={t('predictionLedger.deleteAria').replace(
                      '{statement}',
                      record.statement
                    )}
                    className="shrink-0 rounded-lg border border-stone-200 dark:border-neutral-700 px-2 py-1 text-[11px] text-stone-500 dark:text-neutral-400 hover:bg-stone-50 dark:hover:bg-neutral-800">
                    {t('predictionLedger.deleteButton')}
                  </button>
                )}
              </li>
            ))}
          </ul>
        </section>
      )}

      {records.length === 0 && (
        <div className="py-8 text-center">
          <h3 className="text-sm font-semibold text-stone-700 dark:text-neutral-200">
            {t('predictionLedger.empty')}
          </h3>
          <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
            {t('predictionLedger.emptyHint')}
          </p>
        </div>
      )}
    </div>
  );
};

export default PredictionLedger;
