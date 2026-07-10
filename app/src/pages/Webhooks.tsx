import debug from 'debug';

import SettingsHeader from '../components/settings/components/SettingsHeader';
import { SettingsSection } from '../components/settings/controls';
import { useSettingsNavigation } from '../components/settings/hooks/useSettingsNavigation';
import ComposioTriagePanel from '../components/settings/panels/ComposioTriagePanel';
import Button from '../components/ui/Button';
import ComposeioTriggerHistory from '../components/webhooks/ComposeioTriggerHistory';
import { useComposeioTriggerHistory } from '../hooks/useComposeioTriggerHistory';
import { useT } from '../lib/i18n/I18nContext';

const log = debug('settings:webhooks');

interface WebhooksProps {
  /** When true the page is hosted inside another settings page (the
   *  Integrations tabs) — skip the standalone SettingsHeader chrome and render
   *  the status badge + refresh action inline at the top of the body. */
  embedded?: boolean;
}

export default function Webhooks({ embedded = false }: WebhooksProps) {
  const { t } = useT();
  // [settings] Webhooks renders at /settings/integrations#webhooks (embedded)
  // — the legacy standalone /settings/webhooks-triggers slug redirects there.
  log('rendering with settings shell embedded=%s', embedded);
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const { archiveDir, currentDayFile, entries, loading, error, coreConnected, refresh } =
    useComposeioTriggerHistory(100);

  if (loading && entries.length === 0) {
    return (
      <div className="z-10 relative">
        {!embedded && (
          <SettingsHeader
            title={t('settings.developerMenu.composeioTriggers.title')}
            showBackButton={true}
            onBack={navigateBack}
            breadcrumbs={breadcrumbs}
          />
        )}
        <div className="h-full flex items-center justify-center p-4 pt-6">
          <div className="flex flex-col items-center gap-3">
            <div className="h-8 w-8 animate-spin rounded-full border-2 border-line-strong border-t-primary-500" />
            <span className="text-sm text-content-muted">{t('common.loading')}</span>
          </div>
        </div>
      </div>
    );
  }

  const statusActions = (
    <div className="flex items-center gap-2">
      {/* Bespoke connection status badge — keep intentional visual */}
      <span
        className={`inline-flex items-center gap-1.5 px-2.5 py-1 text-xs font-medium rounded-full ${
          coreConnected ? 'bg-sage-100 text-sage-700' : 'bg-surface-subtle text-content-muted'
        }`}>
        <span
          className={`w-1.5 h-1.5 rounded-full ${
            coreConnected ? 'bg-sage-500' : 'bg-neutral-400 dark:bg-neutral-500'
          }`}
        />
        {coreConnected ? t('skills.connected') : t('skills.disconnect')}
      </span>
      <Button type="button" variant="secondary" size="xs" onClick={() => void refresh()}>
        {t('common.refresh')}
      </Button>
    </div>
  );

  return (
    <div className="z-10 relative">
      {!embedded && (
        <SettingsHeader
          title={t('settings.developerMenu.composeioTriggers.title')}
          showBackButton={true}
          onBack={navigateBack}
          breadcrumbs={breadcrumbs}
          action={statusActions}
        />
      )}

      <div className="p-4 space-y-4">
        {embedded && <div className="flex justify-end">{statusActions}</div>}
        {error && <div className="p-3 rounded-lg bg-coral-50 text-coral-700 text-sm">{error}</div>}

        {/* Archive paths info — bespoke data-display layout, kept as-is */}
        <SettingsSection title={t('skills.search')} description={t('misc.rehydrating')}>
          <div className="px-4 py-3 space-y-2">
            <div>
              <div className="text-xs uppercase tracking-wide text-content-faint">
                {t('webhooks.archiveDirectory')}
              </div>
              <div className="font-mono text-xs break-all text-content">
                {archiveDir ?? t('common.loading')}
              </div>
            </div>
            <div>
              <div className="text-xs uppercase tracking-wide text-content-faint">
                {t('webhooks.todayFile')}
              </div>
              <div className="font-mono text-xs break-all text-content">
                {currentDayFile ?? t('common.loading')}
              </div>
            </div>
          </div>
        </SettingsSection>

        {/* Trigger history — bespoke visualization, kept as-is */}
        <SettingsSection>
          <div className="p-4">
            <ComposeioTriggerHistory entries={entries} />
          </div>
        </SettingsSection>

        {/* Triage settings merged in from the former Integration Triggers
            page so all Composio trigger config lives in one place. */}
        <SettingsSection>
          <ComposioTriagePanel />
        </SettingsSection>
      </div>
    </div>
  );
}
