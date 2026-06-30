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
import {
  SettingsBadge,
  type SettingsBadgeVariant,
  SettingsRow,
  SettingsSection,
  SettingsSwitch,
} from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

const log = debug('privacy-panel');

interface AnnotatedCapability extends Capability {
  privacy: CapabilityPrivacy;
}

const KIND_BADGE_VARIANT: Record<PrivacyDataKind, SettingsBadgeVariant> = {
  raw: 'success',
  derived: 'warning',
  credentials: 'neutral',
  diagnostics: 'primary',
  metadata: 'neutral',
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
    <SettingsPanel
      testId="settings-privacy-panel"
      description={t('pages.settings.account.privacyDesc')}>
      <>
        {/* What leaves my computer */}
        <SettingsSection title={t('privacy.whatLeavesComputer')}>
          {loadState === 'loading' && (
            <p className="p-4 text-xs text-content-muted">{t('privacy.loading')}</p>
          )}
          {loadState === 'error' && (
            <p className="p-4 text-xs text-content-muted" data-testid="privacy-load-error">
              {t('privacy.loadError')}
            </p>
          )}
          {loadState === 'ready' && capabilities.length === 0 && (
            <p className="p-4 text-xs text-content-muted">{t('privacy.noCapabilities')}</p>
          )}
          {loadState === 'ready' && capabilities.length > 0 && (
            <ul data-testid="privacy-capability-list">
              {capabilities.map(cap => (
                <li key={cap.id} className="p-4" data-testid={`privacy-row-${cap.id}`}>
                  <div className="flex items-start justify-between gap-3">
                    <div className="flex-1 min-w-0">
                      <p className="text-sm font-medium text-content">{cap.name}</p>
                      <p className="text-xs text-content-muted mt-1 leading-relaxed">
                        {cap.description}
                      </p>
                      {cap.privacy.destinations.length > 0 && (
                        <p className="text-xs text-content-faint mt-1">
                          {t('privacy.sentTo')}: {cap.privacy.destinations.join(', ')}
                        </p>
                      )}
                    </div>
                    <div className="flex flex-col items-end gap-1 shrink-0">
                      <SettingsBadge variant={KIND_BADGE_VARIANT[cap.privacy.data_kind]}>
                        {kindLabel(cap.privacy.data_kind, t)}
                      </SettingsBadge>
                      <span className="text-[10px] text-content-muted">
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
        </SettingsSection>

        {/* Analytics Section */}
        <SettingsSection title={t('privacy.anonymizedAnalytics')}>
          <SettingsRow
            htmlFor="switch-analytics"
            label={t('privacy.shareAnonymizedData')}
            description={t('privacy.shareAnonymizedDataDesc')}
            control={
              <SettingsSwitch
                id="switch-analytics"
                checked={analyticsEnabled}
                onCheckedChange={() => {
                  void handleToggleAnalytics();
                }}
                data-testid="privacy-analytics-toggle"
              />
            }
          />
        </SettingsSection>

        {/* Meeting Follow-ups Section (#1299) */}
        <SettingsSection title={t('privacy.meetingFollowUps')}>
          <SettingsRow
            htmlFor="switch-meet-handoff"
            label={t('privacy.autoHandoffMeet')}
            description={t('privacy.autoHandoffMeetDesc')}
            control={
              <SettingsSwitch
                id="switch-meet-handoff"
                checked={meetAutoHandoff}
                onCheckedChange={() => {
                  void handleToggleMeetAutoHandoff();
                }}
                aria-label={t('privacy.autoHandoffMeet')}
                data-testid="privacy-meet-handoff-toggle"
              />
            }
          />
        </SettingsSection>

        {/* Info Box */}
        <div className="p-4 bg-surface-muted rounded-xl border border-line">
          <div className="flex items-start space-x-3">
            <svg
              className="w-5 h-5 text-content-faint mt-0.5 flex-shrink-0"
              fill="currentColor"
              viewBox="0 0 20 20">
              <path
                fillRule="evenodd"
                d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7-4a1 1 0 11-2 0 1 1 0 012 0zM9 9a1 1 0 000 2v3a1 1 0 001 1h1a1 1 0 100-2v-3a1 1 0 00-1-1H9z"
                clipRule="evenodd"
              />
            </svg>
            <div>
              <p className="text-xs text-content-muted leading-relaxed">
                {t('privacy.analyticsDisclaimer')}
              </p>
            </div>
          </div>
        </div>
      </>
    </SettingsPanel>
  );
};

export default PrivacyPanel;
