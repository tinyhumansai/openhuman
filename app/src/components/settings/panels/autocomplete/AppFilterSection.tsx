import { useT } from '../../../../lib/i18n/I18nContext';
import type { AutocompleteStatus } from '../../../../utils/tauriCommands';
import Button from '../../../ui/Button';

interface AppFilterSectionProps {
  status: AutocompleteStatus | null;
  isLoading: boolean;
  contextOverride: string;
  focusDebug: string;
  logs: string[];
  message: string | null;
  error: string | null;
  onSetContextOverride: (value: string) => void;
  onRefreshStatus: () => void;
  onStart: () => void;
  onStop: () => void;
  onTestCurrent: () => void;
  onAcceptSuggestion: () => void;
  onDebugFocus: () => void;
  onClearLogs: () => void;
}

const AppFilterSection = ({
  status,
  isLoading,
  contextOverride,
  focusDebug,
  logs,
  message,
  error,
  onSetContextOverride,
  onRefreshStatus,
  onStart,
  onStop,
  onTestCurrent,
  onAcceptSuggestion,
  onDebugFocus,
  onClearLogs,
}: AppFilterSectionProps) => {
  const { t } = useT();
  return (
    <>
      <section className="rounded-2xl border border-line bg-surface p-4 space-y-3">
        <h3 className="text-sm font-semibold text-content">
          {t('settings.autocomplete.appFilter.runtime')}
        </h3>
        <div className="text-sm text-content space-y-1">
          <div>
            {t('settings.autocomplete.appFilter.platformSupported')}:{' '}
            {status?.platform_supported ? t('common.yes') : t('common.no')}
          </div>
          <div>
            {t('settings.autocomplete.appFilter.enabled')}:{' '}
            {status?.enabled ? t('common.yes') : t('common.no')}
          </div>
          <div>
            {t('settings.autocomplete.appFilter.running')}:{' '}
            {status?.running ? t('common.yes') : t('common.no')}
          </div>
          <div>
            {t('settings.autocomplete.appFilter.phase')}:{' '}
            {status?.phase ?? t('settings.autocomplete.shared.unknown')}
          </div>
          <div>
            {t('settings.autocomplete.appFilter.debounce')}:{' '}
            {`${String(status?.debounce_ms ?? 0)}ms`}
          </div>
          <div>
            {t('settings.autocomplete.appFilter.model')}:{' '}
            {status?.model_id ?? t('settings.autocomplete.shared.notApplicable')}
          </div>
          <div>
            {t('settings.autocomplete.appFilter.app')}:{' '}
            {status?.app_name ?? t('settings.autocomplete.shared.notApplicable')}
          </div>
          <div>
            {t('settings.autocomplete.appFilter.lastError')}:{' '}
            {status?.last_error ?? t('settings.autocomplete.shared.none')}
          </div>
          <div>
            {t('settings.autocomplete.appFilter.currentSuggestion')}:{' '}
            {status?.suggestion?.value ?? t('settings.autocomplete.shared.none')}
          </div>
        </div>
        <div className="flex gap-2">
          <Button variant="secondary" size="md" onClick={onRefreshStatus} disabled={isLoading}>
            {isLoading
              ? t('settings.autocomplete.appFilter.refreshing')
              : t('settings.autocomplete.appFilter.refreshStatus')}
          </Button>
          <button
            type="button"
            onClick={onStart}
            disabled={!status?.platform_supported || Boolean(status?.running)}
            className="rounded-lg border border-green-500/60 bg-green-50 dark:bg-green-500/10 px-3 py-2 text-sm text-green-700 dark:text-green-300 disabled:opacity-50">
            {t('autocomplete.start')}
          </button>
          <Button
            variant="secondary"
            tone="danger"
            size="md"
            onClick={onStop}
            disabled={!status?.running}>
            {t('autocomplete.stop')}
          </Button>
        </div>
      </section>

      <section className="rounded-2xl border border-line bg-surface p-4 space-y-3">
        <h3 className="text-sm font-semibold text-content">
          {t('settings.autocomplete.appFilter.test')}
        </h3>
        <div className="space-y-1">
          <div className="text-xs text-content-secondary">
            {t('settings.autocomplete.appFilter.contextOverride')}
          </div>
          <textarea
            value={contextOverride}
            onChange={event => onSetContextOverride(event.target.value)}
            rows={3}
            className="w-full rounded border border-line bg-surface-muted p-2 text-xs text-content"
          />
        </div>
        <div className="flex gap-2">
          <Button variant="primary" size="md" onClick={onTestCurrent}>
            {t('settings.autocomplete.appFilter.getSuggestion')}
          </Button>
          <button
            type="button"
            onClick={onAcceptSuggestion}
            className="rounded-lg border border-emerald-500/60 bg-emerald-50 dark:bg-emerald-500/10 px-3 py-2 text-sm text-emerald-700 dark:text-emerald-300">
            {t('settings.autocomplete.appFilter.acceptSuggestion')}
          </button>
          <button
            type="button"
            onClick={onDebugFocus}
            className="rounded-lg border border-amber-500/60 bg-amber-50 dark:bg-amber-500/10 px-3 py-2 text-sm text-amber-700 dark:text-amber-300">
            {t('settings.autocomplete.appFilter.debugFocus')}
          </button>
        </div>
        {focusDebug && (
          <pre className="max-h-48 overflow-auto rounded-xl border border-line bg-surface-muted p-2 text-xs text-content">
            {focusDebug}
          </pre>
        )}
      </section>

      <section className="rounded-2xl border border-line bg-surface p-4 space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-content">
            {t('settings.autocomplete.appFilter.liveLogs')}
          </h3>
          <Button variant="secondary" size="sm" onClick={onClearLogs}>
            {t('common.clear')}
          </Button>
        </div>
        <pre className="max-h-56 overflow-auto rounded-xl border border-line bg-surface-muted p-2 text-xs text-content">
          {logs.length > 0 ? logs.join('\n') : t('settings.autocomplete.appFilter.noLogs')}
        </pre>
      </section>

      {message && <div className="text-xs text-green-700 dark:text-green-300">{message}</div>}
      {error && <div className="text-xs text-red-600 dark:text-red-300">{error}</div>}
    </>
  );
};

export default AppFilterSection;
