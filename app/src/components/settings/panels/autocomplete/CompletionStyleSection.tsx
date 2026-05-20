import { useT } from '../../../../lib/i18n/I18nContext';
import type { AcceptedCompletion } from '../../../../utils/tauriCommands';

interface CompletionStyleSectionProps {
  enabled: boolean;
  debounceMs: string;
  maxChars: string;
  stylePreset: string;
  styleInstructions: string;
  styleExamplesText: string;
  disabledAppsText: string;
  acceptWithTab: boolean;
  overlayTtlMs: string;
  isSaving: boolean;
  historyEntries: AcceptedCompletion[];
  isHistoryLoading: boolean;
  isClearingHistory: boolean;
  onSetEnabled: (value: boolean) => void;
  onSetDebounceMs: (value: string) => void;
  onSetMaxChars: (value: string) => void;
  onSetStylePreset: (value: string) => void;
  onSetStyleInstructions: (value: string) => void;
  onSetStyleExamplesText: (value: string) => void;
  onSetDisabledAppsText: (value: string) => void;
  onSetAcceptWithTab: (value: boolean) => void;
  onSetOverlayTtlMs: (value: string) => void;
  onSaveConfig: () => void;
  onClearHistory: () => void;
}

const CompletionStyleSection = ({
  enabled,
  debounceMs,
  maxChars,
  stylePreset,
  styleInstructions,
  styleExamplesText,
  disabledAppsText,
  acceptWithTab,
  overlayTtlMs,
  isSaving,
  historyEntries,
  isHistoryLoading,
  isClearingHistory,
  onSetEnabled,
  onSetDebounceMs,
  onSetMaxChars,
  onSetStylePreset,
  onSetStyleInstructions,
  onSetStyleExamplesText,
  onSetDisabledAppsText,
  onSetAcceptWithTab,
  onSetOverlayTtlMs,
  onSaveConfig,
  onClearHistory,
}: CompletionStyleSectionProps) => {
  const { t } = useT();
  return (
    <>
      <section className="rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-4 space-y-3">
        <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
          {t('autocomplete.settings')}
        </h3>
        <label className="flex items-center justify-between rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2">
          <span className="text-sm text-stone-700 dark:text-neutral-200">
            {t('settings.autocomplete.completionStyle.enabled')}
          </span>
          <input
            type="checkbox"
            checked={enabled}
            onChange={event => onSetEnabled(event.target.checked)}
          />
        </label>
        <label className="flex items-center justify-between rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2">
          <span className="text-sm text-stone-700 dark:text-neutral-200">
            {t('autocomplete.acceptWithTab')}
          </span>
          <input
            type="checkbox"
            checked={acceptWithTab}
            onChange={event => onSetAcceptWithTab(event.target.checked)}
          />
        </label>
        <label className="flex items-center justify-between rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2">
          <span className="text-sm text-stone-700 dark:text-neutral-200">
            {t('settings.autocomplete.completionStyle.debounce')}
          </span>
          <input
            type="number"
            min={50}
            max={2000}
            step={10}
            value={debounceMs}
            onChange={event => onSetDebounceMs(event.target.value)}
            className="w-28 rounded border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1 text-xs text-stone-700 dark:text-neutral-200"
          />
        </label>
        <label className="flex items-center justify-between rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2">
          <span className="text-sm text-stone-700 dark:text-neutral-200">
            {t('settings.autocomplete.completionStyle.maxChars')}
          </span>
          <input
            type="number"
            min={32}
            max={1200}
            step={8}
            value={maxChars}
            onChange={event => onSetMaxChars(event.target.value)}
            className="w-28 rounded border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1 text-xs text-stone-700 dark:text-neutral-200"
          />
        </label>
        <label className="flex items-center justify-between rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2">
          <span className="text-sm text-stone-700 dark:text-neutral-200">
            {t('settings.autocomplete.completionStyle.overlayTtl')}
          </span>
          <input
            type="number"
            min={300}
            max={10000}
            step={100}
            value={overlayTtlMs}
            onChange={event => onSetOverlayTtlMs(event.target.value)}
            className="w-28 rounded border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1 text-xs text-stone-700 dark:text-neutral-200"
          />
        </label>
        <label className="flex items-center justify-between rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2">
          <span className="text-sm text-stone-700 dark:text-neutral-200">
            {t('autocomplete.stylePreset')}
          </span>
          <select
            value={stylePreset}
            onChange={event => onSetStylePreset(event.target.value)}
            className="rounded border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-2 py-1 text-xs text-stone-700 dark:text-neutral-200">
            <option value="balanced">Balanced</option>
            <option value="concise">Concise</option>
            <option value="formal">Formal</option>
            <option value="casual">Casual</option>
            <option value="custom">Custom</option>
          </select>
        </label>
        <div className="space-y-1">
          <div className="text-xs text-stone-600 dark:text-neutral-300">
            {t('settings.autocomplete.completionStyle.styleInstructions')}
          </div>
          <textarea
            value={styleInstructions}
            onChange={event => onSetStyleInstructions(event.target.value)}
            rows={3}
            className="w-full rounded border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-2 text-xs text-stone-700 dark:text-neutral-200"
          />
        </div>
        <div className="space-y-1">
          <div className="text-xs text-stone-600 dark:text-neutral-300">
            {t('settings.autocomplete.completionStyle.styleExamples')}
          </div>
          <textarea
            value={styleExamplesText}
            onChange={event => onSetStyleExamplesText(event.target.value)}
            rows={3}
            className="w-full rounded border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-2 text-xs text-stone-700 dark:text-neutral-200"
          />
        </div>
        <div className="space-y-1">
          <div className="text-xs text-stone-600 dark:text-neutral-300">
            {t('autocomplete.disabledApps')}
          </div>
          <textarea
            value={disabledAppsText}
            onChange={event => onSetDisabledAppsText(event.target.value)}
            rows={3}
            className="w-full rounded border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-2 text-xs text-stone-700 dark:text-neutral-200"
          />
        </div>
        <button
          type="button"
          onClick={onSaveConfig}
          disabled={isSaving}
          className="rounded-lg border border-primary-500/60 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-sm text-primary-600 dark:text-primary-300 disabled:opacity-50">
          {isSaving ? t('autocomplete.saving') : t('autocomplete.saveSettings')}
        </button>
      </section>

      <section className="rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-4 space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
            {t('settings.autocomplete.completionStyle.personalizationHistory')}
          </h3>
          <button
            type="button"
            onClick={onClearHistory}
            disabled={isClearingHistory || historyEntries.length === 0}
            className="rounded-lg border border-red-500/60 bg-red-50 dark:bg-red-500/10 px-3 py-1.5 text-xs text-red-600 dark:text-red-300 disabled:opacity-40">
            {isClearingHistory
              ? t('settings.autocomplete.completionStyle.clearing')
              : t('settings.autocomplete.completionStyle.clearHistory')}
          </button>
        </div>
        <p className="text-xs text-stone-500 dark:text-neutral-400">
          {isHistoryLoading
            ? t('common.loading')
            : historyEntries.length === 0
              ? t('settings.autocomplete.completionStyle.noHistory')
              : `${String(historyEntries.length)} ${historyEntries.length === 1 ? t('settings.autocomplete.completionStyle.acceptedCompletion') : t('settings.autocomplete.completionStyle.acceptedCompletions')}`}
        </p>
        {historyEntries.length > 0 && (
          <div className="max-h-48 overflow-y-auto rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-2 space-y-1">
            {historyEntries.map((entry, idx) => (
              <div
                key={`${String(entry.timestamp_ms)}-${String(idx)}`}
                className="flex flex-col gap-0.5 rounded-lg bg-white dark:bg-neutral-900 px-2 py-1.5 text-xs border border-stone-100 dark:border-neutral-800">
                <div className="flex items-center gap-2 text-stone-500 dark:text-neutral-400">
                  <span className="shrink-0">{new Date(entry.timestamp_ms).toLocaleString()}</span>
                  {entry.app_name && (
                    <span className="rounded bg-stone-100 dark:bg-neutral-800 px-1 text-stone-600 dark:text-neutral-300">
                      {entry.app_name}
                    </span>
                  )}
                </div>
                <div className="flex items-baseline gap-1 text-stone-700 dark:text-neutral-200 truncate">
                  <span className="shrink-0 text-stone-400 dark:text-neutral-500">…</span>
                  <span className="truncate text-stone-500 dark:text-neutral-400">
                    {entry.context.slice(-40)}
                  </span>
                  <span className="shrink-0 text-stone-400 dark:text-neutral-500">→</span>
                  <span className="font-medium text-primary-500 truncate">{entry.suggestion}</span>
                </div>
              </div>
            ))}
          </div>
        )}
      </section>
    </>
  );
};

export default CompletionStyleSection;
