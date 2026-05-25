import debug from 'debug';
import { useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { useCoreState } from '../../../providers/CoreStateProvider';
import {
  type Capability,
  type CapabilityPrivacy,
  listCapabilities,
  type PrivacyDataKind,
} from '../../../utils/tauriCommands/aboutApp';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const log = debug('privacy-panel');

interface AnnotatedCapability extends Capability {
  privacy: CapabilityPrivacy;
}

const KIND_BADGE_CLASS: Record<PrivacyDataKind, string> = {
  raw: 'bg-sage-50 dark:bg-sage-500/10 text-sage-700 dark:text-sage-300 border-sage-200 dark:border-sage-500/30',
  derived:
    'bg-amber-50 dark:bg-amber-500/10 text-amber-700 dark:text-amber-300 border-amber-200 dark:border-amber-500/30',
  credentials:
    'bg-stone-100 dark:bg-neutral-800 text-stone-700 dark:text-neutral-200 border-stone-200 dark:border-neutral-800',
  diagnostics:
    'bg-primary-50 dark:bg-primary-500/10 text-primary-700 dark:text-primary-300 border-primary-200 dark:border-primary-500/30',
  metadata:
    'bg-stone-50 dark:bg-neutral-800/60 text-stone-600 dark:text-neutral-300 border-stone-200 dark:border-neutral-800',
};

function kindLabel(kind: PrivacyDataKind, t: (key: string) => string): string {
  switch (kind) {
    case 'raw':
      return t('privacy.dataKind.raw');
    case 'derived':
      return t('privacy.dataKind.derived');
    case 'credentials':
      return t('privacy.dataKind.credentials');
    case 'diagnostics':
      return t('privacy.dataKind.diagnostics');
    case 'metadata':
      return t('privacy.dataKind.metadata');
  }
}

const PrivacyPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const { snapshot, setAnalyticsEnabled, setMeetAutoOrchestratorHandoff } = useCoreState();
  const analyticsEnabled = snapshot.analyticsEnabled;
  const meetAutoHandoff = snapshot.meetAutoOrchestratorHandoff;
  const { t } = useT();

  const [capabilities, setCapabilities] = useState<AnnotatedCapability[]>([]);
  const [loadState, setLoadState] = useState<'loading' | 'ready' | 'error'>('loading');

  useEffect(() => {
    let cancelled = false;
    log('[privacy] fetching capability catalog');
    listCapabilities()
      .then(items => {
        if (cancelled) return;
        const annotated = items.filter(
          (c): c is AnnotatedCapability => c.privacy !== undefined && c.privacy !== null
        );
        log('[privacy] catalog ready', { total: items.length, annotated: annotated.length });
        setCapabilities(annotated);
        setLoadState('ready');
      })
      .catch(err => {
        if (cancelled) return;
        console.warn('[privacy] failed to load capability catalog:', err);
        setLoadState('error');
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const handleToggleAnalytics = async () => {
    const newValue = !analyticsEnabled;
    try {
      await setAnalyticsEnabled(newValue);
    } catch (error) {
      console.warn('[privacy] failed to persist analytics setting:', error);
    }
  };

  const handleToggleMeetAutoHandoff = async () => {
    const newValue = !meetAutoHandoff;
    try {
      await setMeetAutoOrchestratorHandoff(newValue);
    } catch (error) {
      console.warn('[privacy] failed to persist meet auto-handoff setting:', error);
    }
  };

  return (
    <div data-testid="settings-privacy-panel">
      <SettingsHeader
        title={t('privacy.title')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div>
        <div className="p-4 space-y-4">
          {/* What leaves my computer */}
          <div>
            <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-400 dark:text-neutral-500 mb-3 px-1">
              {t('privacy.whatLeavesComputer')}
            </h3>
            <div className="bg-white dark:bg-neutral-900 rounded-xl border border-stone-200 dark:border-neutral-800 overflow-hidden">
              {loadState === 'loading' && (
                <p className="p-4 text-xs text-stone-500 dark:text-neutral-400">
                  {t('privacy.loading')}
                </p>
              )}
              {loadState === 'error' && (
                <p
                  className="p-4 text-xs text-stone-500 dark:text-neutral-400"
                  data-testid="privacy-load-error">
                  {t('privacy.loadError')}
                </p>
              )}
              {loadState === 'ready' && capabilities.length === 0 && (
                <p className="p-4 text-xs text-stone-500 dark:text-neutral-400">
                  {t('privacy.noCapabilities')}
                </p>
              )}
              {loadState === 'ready' && capabilities.length > 0 && (
                <ul
                  className="divide-y divide-stone-100 dark:divide-neutral-800"
                  data-testid="privacy-capability-list">
                  {capabilities.map(cap => (
                    <li key={cap.id} className="p-4" data-testid={`privacy-row-${cap.id}`}>
                      <div className="flex items-start justify-between gap-3">
                        <div className="flex-1 min-w-0">
                          <p className="text-sm font-medium text-stone-900 dark:text-neutral-100">
                            {cap.name}
                          </p>
                          <p className="text-xs text-stone-500 dark:text-neutral-400 mt-1 leading-relaxed">
                            {cap.description}
                          </p>
                          {cap.privacy.destinations.length > 0 && (
                            <p className="text-xs text-stone-400 dark:text-neutral-500 mt-1">
                              {t('privacy.sentTo')}: {cap.privacy.destinations.join(', ')}
                            </p>
                          )}
                        </div>
                        <div className="flex flex-col items-end gap-1 shrink-0">
                          <span
                            className={`text-[10px] uppercase tracking-wider px-2 py-0.5 rounded border ${KIND_BADGE_CLASS[cap.privacy.data_kind]}`}>
                            {kindLabel(cap.privacy.data_kind, t)}
                          </span>
                          <span className="text-[10px] text-stone-500 dark:text-neutral-400">
                            {cap.privacy.leaves_device
                              ? t('privacy.leavesDevice')
                              : t('privacy.staysLocal')}
                          </span>
                        </div>
                      </div>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </div>

          {/* Analytics Section */}
          <div>
            <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-400 dark:text-neutral-500 mb-3 px-1">
              {t('privacy.anonymizedAnalytics')}
            </h3>
            <div className="bg-white dark:bg-neutral-900 rounded-xl border border-stone-200 dark:border-neutral-800 overflow-hidden">
              <div className="flex items-center justify-between p-4">
                <div className="flex-1 mr-4">
                  <p className="text-sm font-medium text-stone-900 dark:text-neutral-100">
                    {t('privacy.shareAnonymizedData')}
                  </p>
                  <p className="text-xs text-stone-500 dark:text-neutral-400 mt-1 leading-relaxed">
                    {t('privacy.shareAnonymizedDataDesc')}
                  </p>
                </div>
                <button
                  type="button"
                  onClick={handleToggleAnalytics}
                  data-testid="privacy-analytics-toggle"
                  className={`relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
                    analyticsEnabled ? 'bg-primary-500' : 'bg-stone-600 dark:bg-neutral-600'
                  }`}
                  role="switch"
                  aria-checked={analyticsEnabled}>
                  <span
                    className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white dark:bg-neutral-900 shadow ring-0 transition duration-200 ease-in-out ${
                      analyticsEnabled ? 'translate-x-5' : 'translate-x-0'
                    }`}
                  />
                </button>
              </div>
            </div>
          </div>

          {/* Meeting Follow-ups Section (#1299) */}
          <div>
            <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-400 dark:text-neutral-500 mb-3 px-1">
              {t('privacy.meetingFollowUps')}
            </h3>
            <div className="bg-white dark:bg-neutral-900 rounded-xl border border-stone-200 dark:border-neutral-800 overflow-hidden">
              <div className="flex items-center justify-between p-4">
                <div className="flex-1 mr-4">
                  <p className="text-sm font-medium text-stone-900 dark:text-neutral-100">
                    {t('privacy.autoHandoffMeet')}
                  </p>
                  <p className="text-xs text-stone-500 dark:text-neutral-400 mt-1 leading-relaxed">
                    {t('privacy.autoHandoffMeetDesc')}
                  </p>
                </div>
                <button
                  data-testid="privacy-meet-handoff-toggle"
                  onClick={handleToggleMeetAutoHandoff}
                  aria-label={t('privacy.autoHandoffMeet')}
                  className={`relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
                    meetAutoHandoff ? 'bg-primary-500' : 'bg-stone-600 dark:bg-neutral-600'
                  }`}
                  role="switch"
                  aria-checked={meetAutoHandoff}>
                  <span
                    className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white dark:bg-neutral-900 shadow ring-0 transition duration-200 ease-in-out ${
                      meetAutoHandoff ? 'translate-x-5' : 'translate-x-0'
                    }`}
                  />
                </button>
              </div>
            </div>
          </div>

          {/* Info Box */}
          <div className="p-4 bg-stone-50 dark:bg-neutral-800/60 rounded-xl border border-stone-200 dark:border-neutral-800">
            <div className="flex items-start space-x-3">
              <svg
                className="w-5 h-5 text-stone-400 dark:text-neutral-500 mt-0.5 flex-shrink-0"
                fill="currentColor"
                viewBox="0 0 20 20">
                <path
                  fillRule="evenodd"
                  d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7-4a1 1 0 11-2 0 1 1 0 012 0zM9 9a1 1 0 000 2v3a1 1 0 001 1h1a1 1 0 100-2v-3a1 1 0 00-1-1H9z"
                  clipRule="evenodd"
                />
              </svg>
              <div>
                <p className="text-xs text-stone-500 dark:text-neutral-400 leading-relaxed">
                  {t('privacy.analyticsDisclaimer')}
                </p>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default PrivacyPanel;
