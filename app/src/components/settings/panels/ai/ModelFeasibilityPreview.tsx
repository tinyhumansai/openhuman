/** Read-only cost/capability estimator. Never changes model routing or secrets. */
import { useMemo, useState } from 'react';

import { useT } from '../../../../lib/i18n/I18nContext';
import type { ModelRegistryEntry } from '../../../../utils/tauriCommands/config';
import {
  assessModelFeasibility,
  formatEstimatedModelCost,
  type ModelFitReason,
  type ModelFitStatus,
} from './modelFeasibility';

const PAGE_SIZE = 8;

const reasonLabels: Record<ModelFitReason, [string, string]> = {
  context_too_small: [
    'settings.ai.feasibility.contextTooSmall',
    'Context smaller than input + output tokens',
  ],
  vision_unavailable: [
    'settings.ai.feasibility.visionUnavailable',
    'Vision unsupported in saved metadata',
  ],
  estimated_cost_exceeds_budget: [
    'settings.ai.feasibility.overBudget',
    'Estimated cost exceeds budget',
  ],
  unknown_context: ['settings.ai.feasibility.unknownContext', 'Context size unknown'],
  unknown_vision: ['settings.ai.feasibility.unknownVision', 'Vision capability unknown'],
  unknown_price: ['settings.ai.feasibility.unknownPrice', 'Price missing, zero or invalid'],
};

interface Props {
  models: readonly ModelRegistryEntry[];
}

export function ModelFeasibilityPreview({ models }: Props) {
  const { t } = useT();
  const [inputText, setInputText] = useState('32000');
  const [cachedInputText, setCachedInputText] = useState('0');
  const [outputText, setOutputText] = useState('2000');
  const [budgetText, setBudgetText] = useState('');
  const [requiresVision, setRequiresVision] = useState(false);
  const [query, setQuery] = useState('');
  const [fitStatus, setFitStatus] = useState<'all' | ModelFitStatus>('all');
  const [visibleCount, setVisibleCount] = useState(PAGE_SIZE);

  const report = useMemo(() => {
    if (!inputText.trim() || !outputText.trim()) return null;
    try {
      return assessModelFeasibility(models, {
        inputTokens: Number(inputText),
        cachedInputTokens: cachedInputText.trim() === '' ? 0 : Number(cachedInputText),
        outputTokens: Number(outputText),
        requiresVision,
        maxEstimatedCostUSD: budgetText.trim() === '' ? null : Number(budgetText),
      });
    } catch {
      return null;
    }
  }, [models, inputText, cachedInputText, outputText, budgetText, requiresVision]);

  const matching = useMemo(() => {
    if (!report) return [];
    const needle = query.trim().toLocaleLowerCase();
    return report.rows.filter(
      row =>
        (fitStatus === 'all' || row.status === fitStatus) &&
        (!needle || (row.model.provider + '/' + row.model.id).toLocaleLowerCase().includes(needle))
    );
  }, [report, query, fitStatus]);

  const displayed = matching.slice(0, visibleCount);
  const cachedExceedsInput =
    inputText.trim() !== '' &&
    cachedInputText.trim() !== '' &&
    Number(cachedInputText) > Number(inputText);

  return (
    <section
      aria-label={t('settings.ai.feasibility.aria', 'Model suitability preview')}
      className="space-y-3 rounded-lg border border-line-subtle p-3">
      <div>
        <h4 className="text-sm font-medium text-content">
          {t('settings.ai.feasibility.title', 'Model suitability and cost preview')}
        </h4>
        <p className="text-xs text-content-muted">
          {t(
            'settings.ai.feasibility.advisory',
            'Based on saved metadata, not verified live prices, quotas, keys or availability. This is advisory only and does not change any workload route.'
          )}
        </p>
      </div>
      <div className="grid grid-cols-2 gap-2 md:grid-cols-3 lg:grid-cols-5">
        <label className="space-y-1 text-xs text-content-secondary">
          <span>{t('settings.ai.feasibility.input', 'Estimated input tokens')}</span>
          <input
            type="number"
            min="0"
            step="1"
            value={inputText}
            aria-label={t('settings.ai.feasibility.input', 'Estimated input tokens')}
            onChange={e => setInputText(e.target.value)}
            className="w-full rounded border border-line-subtle bg-transparent px-2 py-1 text-content"
          />
        </label>
        <label className="space-y-1 text-xs text-content-secondary">
          <span>{t('settings.ai.feasibility.cached', 'Cached input tokens (within input)')}</span>
          <input
            type="number"
            min="0"
            max={inputText}
            step="1"
            value={cachedInputText}
            aria-label={t('settings.ai.feasibility.cached', 'Cached input tokens (within input)')}
            onChange={e => setCachedInputText(e.target.value)}
            className="w-full rounded border border-line-subtle bg-transparent px-2 py-1 text-content"
          />
        </label>
        <label className="space-y-1 text-xs text-content-secondary">
          <span>{t('settings.ai.feasibility.output', 'Estimated output tokens')}</span>
          <input
            type="number"
            min="0"
            step="1"
            value={outputText}
            aria-label={t('settings.ai.feasibility.output', 'Estimated output tokens')}
            onChange={e => setOutputText(e.target.value)}
            className="w-full rounded border border-line-subtle bg-transparent px-2 py-1 text-content"
          />
        </label>
        <label className="space-y-1 text-xs text-content-secondary">
          <span>{t('settings.ai.feasibility.budget', 'Cost limit (USD, optional)')}</span>
          <input
            type="number"
            min="0"
            step="0.01"
            value={budgetText}
            aria-label={t('settings.ai.feasibility.budgetAria', 'Estimated cost limit in USD')}
            placeholder={t('settings.ai.feasibility.noLimit', 'No limit')}
            onChange={e => setBudgetText(e.target.value)}
            className="w-full rounded border border-line-subtle bg-transparent px-2 py-1 text-content"
          />
        </label>
        <label className="flex items-center gap-2 text-xs text-content-secondary">
          <input
            type="checkbox"
            checked={requiresVision}
            onChange={e => setRequiresVision(e.target.checked)}
          />
          <span>{t('settings.ai.feasibility.requireVision', 'Require vision')}</span>
        </label>
      </div>
      {!report ? (
        <p role="alert" className="text-xs text-content-secondary">
          {cachedExceedsInput
            ? t(
                'settings.ai.feasibility.invalidCached',
                'Cached input must be within total input tokens.'
              )
            : t(
                'settings.ai.feasibility.invalidCounts',
                'Enter non-negative whole token counts (cached input is a subset) and an optional non-negative cost limit.'
              )}
        </p>
      ) : (
        <>
          <p role="status" className="text-xs text-content-secondary">
            {t('settings.ai.feasibility.fitCount', 'Saved-metadata fit')}:{' '}
            {report.counts.metadata_fit}; {t('settings.ai.feasibility.unknownCount', 'unknown')}:{' '}
            {report.counts.unknown};{' '}
            {t('settings.ai.feasibility.excludedCount', 'excluded by supplied constraints')}:{' '}
            {report.counts.excluded}.
          </p>
          <div className="grid gap-2 sm:grid-cols-2">
            <label className="space-y-1 text-xs text-content-secondary">
              <span>{t('settings.ai.feasibility.search', 'Search suitability results')}</span>
              <input
                type="search"
                value={query}
                aria-label={t('settings.ai.feasibility.search', 'Search suitability results')}
                onChange={e => {
                  setQuery(e.target.value);
                  setVisibleCount(PAGE_SIZE);
                }}
                className="w-full rounded border border-line-subtle bg-transparent px-2 py-1 text-content"
              />
            </label>
            <label className="space-y-1 text-xs text-content-secondary">
              <span>{t('settings.ai.feasibility.filter', 'Filter suitability status')}</span>
              <select
                value={fitStatus}
                aria-label={t('settings.ai.feasibility.filter', 'Filter suitability status')}
                onChange={e => {
                  setFitStatus(e.target.value as 'all' | ModelFitStatus);
                  setVisibleCount(PAGE_SIZE);
                }}
                className="w-full rounded border border-line-subtle bg-surface px-2 py-1 text-content">
                <option value="all">{t('settings.ai.feasibility.all', 'All statuses')}</option>
                <option value="metadata_fit">
                  {t('settings.ai.feasibility.fits', 'Metadata fit')}
                </option>
                <option value="unknown">
                  {t('settings.ai.feasibility.unknown', 'Unknown eligibility')}
                </option>
                <option value="excluded">
                  {t('settings.ai.feasibility.excluded', 'Excluded')}
                </option>
              </select>
            </label>
          </div>
          <div className="max-h-60 space-y-1 overflow-y-auto" aria-live="polite">
            {displayed.map(({ model, estimatedCostUSD, reasons, status }) => (
              <div
                key={model.provider + '\0' + model.id}
                className="flex flex-wrap items-start justify-between gap-2 border-b border-line-subtle py-1.5 text-xs">
                <div className="min-w-0 flex-1">
                  <div className="break-all font-medium text-content">
                    {model.provider}/{model.id}
                  </div>
                  <div className="text-content-muted">
                    {status === 'metadata_fit'
                      ? t(
                          'settings.ai.feasibility.fitDescription',
                          'Matches saved metadata; availability unverified'
                        )
                      : status === 'unknown'
                        ? t('settings.ai.feasibility.unknown', 'Unknown eligibility')
                        : t(
                            'settings.ai.feasibility.excludedDescription',
                            'Excluded by constraints'
                          )}
                    {reasons.length > 0 &&
                      ': ' + reasons.map(reason => t(...reasonLabels[reason])).join('; ')}
                  </div>
                </div>
                <div className="shrink-0 text-content-secondary">
                  {estimatedCostUSD === null
                    ? t('settings.ai.feasibility.costUnknown', 'Cost unknown')
                    : formatEstimatedModelCost(estimatedCostUSD)}
                </div>
              </div>
            ))}
            {matching.length === 0 && (
              <p className="text-xs text-content-muted">
                {report.rows.length === 0
                  ? t('settings.ai.feasibility.empty', 'No saved models to evaluate.')
                  : t('settings.ai.feasibility.noMatch', 'No models match these filters.')}
              </p>
            )}
          </div>
          <p className="text-xs text-content-muted" aria-live="polite">
            {t('settings.ai.feasibility.showing', 'Showing')} {displayed.length}{' '}
            {t('settings.ai.feasibility.of', 'of')} {matching.length}{' '}
            {t('settings.ai.feasibility.matching', 'matching model entries')}.
          </p>
          {displayed.length < matching.length && (
            <button
              type="button"
              onClick={() => setVisibleCount(count => Math.min(matching.length, count + PAGE_SIZE))}
              className="rounded-md border border-line px-3 py-1.5 text-xs text-content hover:bg-surface-subtle">
              {t('settings.ai.feasibility.more', 'Show more models')}
            </button>
          )}
        </>
      )}
    </section>
  );
}
