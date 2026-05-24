import { useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { type AISettings, loadAISettings } from '../../../services/api/aiSettingsApi';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import { BackgroundLoopControls } from './AIPanel';

const HeartbeatPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const [snapshot, setSnapshot] = useState<AISettings | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    loadAISettings()
      .then(s => {
        if (!cancelled) setSnapshot(s);
      })
      .catch(err => {
        if (!cancelled) setLoadError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title={t('settings.heartbeat.title')}
        showBackButton
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />
      <div className="p-4">
        {loadError && (
          <div className="mb-3 rounded-md border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-xs text-coral-700 dark:text-coral-300">
            {loadError}
          </div>
        )}
        {snapshot ? (
          <BackgroundLoopControls
            view="heartbeat"
            hideHeader
            routing={snapshot.routing}
            cloudProviders={snapshot.cloudProviders}
          />
        ) : (
          <div className="text-xs text-stone-500 dark:text-neutral-400">{t('common.loading')}</div>
        )}
      </div>
    </div>
  );
};

export default HeartbeatPanel;
