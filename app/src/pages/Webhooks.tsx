import ComposeioTriggerHistory from '../components/webhooks/ComposeioTriggerHistory';
import { useComposeioTriggerHistory } from '../hooks/useComposeioTriggerHistory';
import { useT } from '../lib/i18n/I18nContext';

export default function Webhooks() {
  const { t } = useT();
  const { archiveDir, currentDayFile, entries, loading, error, coreConnected, refresh } =
    useComposeioTriggerHistory(100);

  if (loading && entries.length === 0) {
    return (
      <div className="h-full flex items-center justify-center p-4 pt-6">
        <div className="flex flex-col items-center gap-3">
          <div className="h-8 w-8 animate-spin rounded-full border-2 border-stone-300 dark:border-neutral-700 border-t-primary-500" />
          <span className="text-sm text-stone-500 dark:text-neutral-400">
            {t('common.loading')}
          </span>
        </div>
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto p-4 pt-6">
      <div className="max-w-2xl mx-auto space-y-4">
        {/* Connection status */}
        <div className="flex flex-wrap items-center gap-3">
          <h2 className="text-xl font-semibold text-stone-900 dark:text-neutral-100">
            {t('skills.integrations')}
          </h2>
          <span
            className={`inline-flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded-full ${
              coreConnected
                ? 'bg-sage-100 text-sage-700'
                : 'bg-stone-100 dark:bg-neutral-800 text-stone-500 dark:text-neutral-400'
            }`}>
            <span
              className={`w-1.5 h-1.5 rounded-full ${
                coreConnected ? 'bg-sage-500' : 'bg-stone-400 dark:bg-neutral-500'
              }`}
            />
            {coreConnected ? t('skills.connected') : t('skills.disconnect')}
          </span>
          <button
            type="button"
            onClick={() => void refresh()}
            className="rounded-full border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-1.5 text-xs font-medium text-stone-700 dark:text-neutral-200 transition hover:border-stone-300 dark:hover:border-neutral-700 hover:bg-stone-50 dark:hover:bg-neutral-800/60">
            {t('common.refresh')}
          </button>
        </div>

        {error && <div className="p-3 rounded-lg bg-coral-50 text-coral-700 text-sm">{error}</div>}

        <div className="bg-white dark:bg-neutral-900 rounded-2xl shadow-soft border border-stone-200 dark:border-neutral-800 p-6">
          <div className="space-y-3">
            <h3 className="text-lg font-semibold text-stone-900 dark:text-neutral-100">
              {t('skills.search')}
            </h3>
            <p className="text-sm text-stone-600 dark:text-neutral-300">{t('misc.rehydrating')}</p>
            <div className="space-y-2 rounded-2xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-4">
              <div>
                <div className="text-xs uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                  {t('webhooks.archiveDirectory')}
                </div>
                <div className="font-mono text-xs break-all text-stone-700 dark:text-neutral-200">
                  {archiveDir ?? t('common.loading')}
                </div>
              </div>
              <div>
                <div className="text-xs uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                  {t('webhooks.todayFile')}
                </div>
                <div className="font-mono text-xs break-all text-stone-700 dark:text-neutral-200">
                  {currentDayFile ?? t('common.loading')}
                </div>
              </div>
            </div>
          </div>
        </div>

        <div className="bg-white dark:bg-neutral-900 rounded-2xl shadow-soft border border-stone-200 dark:border-neutral-800 p-6">
          <ComposeioTriggerHistory entries={entries} />
        </div>
      </div>
    </div>
  );
}
