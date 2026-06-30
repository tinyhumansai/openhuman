import { useT } from '../../../../lib/i18n/I18nContext';
import type { AcceptedCompletion } from '../../../../utils/tauriCommands';
import Button from '../../../ui/Button';

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
      <section className="rounded-2xl border border-line bg-surface p-4 space-y-3">
        <h3 className="text-sm font-semibold text-content">{t('autocomplete.settings')}</h3>
        <label className="flex items-center justify-between rounded-xl border border-line bg-surface-muted px-3 py-2">
          <span className="text-sm text-content">
            {t('settings.autocomplete.completionStyle.enabled')}
          </span>
          <input
            type="checkbox"
            checked={enabled}
            onChange={event => onSetEnabled(event.target.checked)}
          />
        </label>
        <label className="flex items-center justify-between rounded-xl border border-line bg-surface-muted px-3 py-2">
          <span className="text-sm text-content">{t('autocomplete.acceptWithTab')}</span>
          <input
            type="checkbox"
            checked={acceptWithTab}
            onChange={event => onSetAcceptWithTab(event.target.checked)}
          />
        </label>
        <label className="flex items-center justify-between rounded-xl border border-line bg-surface-muted px-3 py-2">
          <span className="text-sm text-content">
            {t('settings.autocomplete.completionStyle.debounce')}
          </span>
          <input
            type="number"
            min={50}
            max={2000}
            step={10}
            value={debounceMs}
            onChange={event => onSetDebounceMs(event.target.value)}
            className="w-28 rounded border border-line-strong bg-surface px-2 py-1 text-xs text-content"
          />
        </label>
        <label className="flex items-center justify-between rounded-xl border border-line bg-surface-muted px-3 py-2">
          <span className="text-sm text-content">
            {t('settings.autocomplete.completionStyle.maxChars')}
          </span>
          <input
            type="number"
            min={32}
            max={1200}
            step={8}
            value={maxChars}
            onChange={event => onSetMaxChars(event.target.value)}
            className="w-28 rounded border border-line-strong bg-surface px-2 py-1 text-xs text-content"
          />
        </label>
        <label className="flex items-center justify-between rounded-xl border border-line bg-surface-muted px-3 py-2">
          <span className="text-sm text-content">
            {t('settings.autocomplete.completionStyle.overlayTtl')}
          </span>
          <input
            type="number"
            min={300}
            max={10000}
            step={100}
            value={overlayTtlMs}
            onChange={event => onSetOverlayTtlMs(event.target.value)}
            className="w-28 rounded border border-line-strong bg-surface px-2 py-1 text-xs text-content"
          />
        </label>
        <label className="flex items-center justify-between rounded-xl border border-line bg-surface-muted px-3 py-2">
          <span className="text-sm text-content">{t('autocomplete.stylePreset')}</span>
          <select
            value={stylePreset}
            onChange={event => onSetStylePreset(event.target.value)}
            className="rounded border border-line-strong bg-surface px-2 py-1 text-xs text-content">
            <option value="balanced">{t('autocomplete.style.balanced')}</option>
            <option value="concise">{t('autocomplete.style.concise')}</option>
            <option value="formal">{t('autocomplete.style.formal')}</option>
            <option value="casual">{t('autocomplete.style.casual')}</option>
            <option value="custom">{t('autocomplete.style.custom')}</option>
          </select>
        </label>
        <div className="space-y-1">
          <div className="text-xs text-content-secondary">
            {t('settings.autocomplete.completionStyle.styleInstructions')}
          </div>
          <textarea
            value={styleInstructions}
            onChange={event => onSetStyleInstructions(event.target.value)}
            rows={3}
            className="w-full rounded border border-line bg-surface-muted p-2 text-xs text-content"
          />
        </div>
        <div className="space-y-1">
          <div className="text-xs text-content-secondary">
            {t('settings.autocomplete.completionStyle.styleExamples')}
          </div>
          <textarea
            value={styleExamplesText}
            onChange={event => onSetStyleExamplesText(event.target.value)}
            rows={3}
            className="w-full rounded border border-line bg-surface-muted p-2 text-xs text-content"
          />
        </div>
        <div className="space-y-1">
          <div className="text-xs text-content-secondary">{t('autocomplete.disabledApps')}</div>
          <textarea
            value={disabledAppsText}
            onChange={event => onSetDisabledAppsText(event.target.value)}
            rows={3}
            className="w-full rounded border border-line bg-surface-muted p-2 text-xs text-content"
          />
        </div>
        <Button variant="primary" size="sm" onClick={onSaveConfig} disabled={isSaving}>
          {isSaving ? t('autocomplete.saving') : t('autocomplete.saveSettings')}
        </Button>
      </section>

      <section className="rounded-2xl border border-line bg-surface p-4 space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-content">
            {t('settings.autocomplete.completionStyle.personalizationHistory')}
          </h3>
          <Button
            variant="secondary"
            tone="danger"
            size="sm"
            onClick={onClearHistory}
            disabled={isClearingHistory || historyEntries.length === 0}>
            {isClearingHistory
              ? t('settings.autocomplete.completionStyle.clearing')
              : t('settings.autocomplete.completionStyle.clearHistory')}
          </Button>
        </div>
        <p className="text-xs text-content-muted">
          {isHistoryLoading
            ? t('common.loading')
            : historyEntries.length === 0
              ? t('settings.autocomplete.completionStyle.noHistory')
              : (historyEntries.length === 1
                  ? t('settings.autocomplete.completionStyle.acceptedCompletion')
                  : t('settings.autocomplete.completionStyle.acceptedCompletions')
                ).replace('{count}', String(historyEntries.length))}
        </p>
        {historyEntries.length > 0 && (
          <div className="max-h-48 overflow-y-auto rounded-xl border border-line bg-surface-muted p-2 space-y-1">
            {historyEntries.map((entry, idx) => (
              <div
                key={`${String(entry.timestamp_ms)}-${String(idx)}`}
                className="flex flex-col gap-0.5 rounded-lg bg-surface px-2 py-1.5 text-xs border border-line-subtle">
                <div className="flex items-center gap-2 text-content-muted">
                  <span className="shrink-0">{new Date(entry.timestamp_ms).toLocaleString()}</span>
                  {entry.app_name && (
                    <span className="rounded bg-surface-subtle px-1 text-content-secondary">
                      {entry.app_name}
                    </span>
                  )}
                </div>
                <div className="flex items-baseline gap-1 text-content truncate">
                  <span className="shrink-0 text-content-faint">…</span>
                  <span className="truncate text-content-muted">{entry.context.slice(-40)}</span>
                  <span className="shrink-0 text-content-faint">→</span>
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
