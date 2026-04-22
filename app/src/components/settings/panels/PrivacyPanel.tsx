import debug from 'debug';
import { useEffect, useState } from 'react';

import { useCoreState } from '../../../providers/CoreStateProvider';
import {
  type Capability,
  type CapabilityPrivacy,
  type PrivacyDataKind,
  listCapabilities,
} from '../../../utils/tauriCommands/aboutApp';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const log = debug('privacy-panel');

interface AnnotatedCapability extends Capability {
  privacy: CapabilityPrivacy;
}

const KIND_LABEL: Record<PrivacyDataKind, string> = {
  raw: 'Raw user content',
  derived: 'Derived signals',
  credentials: 'Credentials',
  diagnostics: 'Diagnostics',
  metadata: 'Metadata',
};

const KIND_BADGE_CLASS: Record<PrivacyDataKind, string> = {
  raw: 'bg-sage-50 text-sage-700 border-sage-200',
  derived: 'bg-amber-50 text-amber-700 border-amber-200',
  credentials: 'bg-stone-100 text-stone-700 border-stone-200',
  diagnostics: 'bg-primary-50 text-primary-700 border-primary-200',
  metadata: 'bg-stone-50 text-stone-600 border-stone-200',
};

const PrivacyPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const { snapshot, setAnalyticsEnabled } = useCoreState();
  const analyticsEnabled = snapshot.analyticsEnabled;

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

  return (
    <div>
      <SettingsHeader
        title="Privacy & Security"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div>
        <div className="p-4 space-y-4">
          {/* What leaves my computer */}
          <div>
            <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-400 mb-3 px-1">
              What leaves your computer
            </h3>
            <div className="bg-white rounded-xl border border-stone-200 overflow-hidden">
              {loadState === 'loading' && (
                <p className="p-4 text-xs text-stone-500">Loading privacy details&hellip;</p>
              )}
              {loadState === 'error' && (
                <p
                  className="p-4 text-xs text-stone-500"
                  data-testid="privacy-load-error">
                  Couldn&rsquo;t load the live privacy list. Analytics controls below still work.
                </p>
              )}
              {loadState === 'ready' && capabilities.length === 0 && (
                <p className="p-4 text-xs text-stone-500">
                  No capabilities currently disclose data movement.
                </p>
              )}
              {loadState === 'ready' && capabilities.length > 0 && (
                <ul
                  className="divide-y divide-stone-100"
                  data-testid="privacy-capability-list">
                  {capabilities.map(cap => (
                    <li
                      key={cap.id}
                      className="p-4"
                      data-testid={`privacy-row-${cap.id}`}>
                      <div className="flex items-start justify-between gap-3">
                        <div className="flex-1 min-w-0">
                          <p className="text-sm font-medium text-stone-900">{cap.name}</p>
                          <p className="text-xs text-stone-500 mt-1 leading-relaxed">
                            {cap.description}
                          </p>
                          {cap.privacy.destinations.length > 0 && (
                            <p className="text-xs text-stone-400 mt-1">
                              Sent to: {cap.privacy.destinations.join(', ')}
                            </p>
                          )}
                        </div>
                        <div className="flex flex-col items-end gap-1 shrink-0">
                          <span
                            className={`text-[10px] uppercase tracking-wider px-2 py-0.5 rounded border ${KIND_BADGE_CLASS[cap.privacy.data_kind]}`}>
                            {KIND_LABEL[cap.privacy.data_kind]}
                          </span>
                          <span className="text-[10px] text-stone-500">
                            {cap.privacy.leaves_device ? 'Leaves device' : 'Stays local'}
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
            <h3 className="text-xs font-semibold uppercase tracking-wider text-stone-400 mb-3 px-1">
              Anonymized Analytics
            </h3>
            <div className="bg-white rounded-xl border border-stone-200 overflow-hidden">
              <div className="flex items-center justify-between p-4">
                <div className="flex-1 mr-4">
                  <p className="text-sm font-medium text-stone-900">Share Anonymized Usage Data</p>
                  <p className="text-xs text-stone-500 mt-1 leading-relaxed">
                    Help improve OpenHuman by sharing anonymous crash reports and usage analytics.
                    All data is fully anonymized &mdash; no personal data, messages, wallet keys, or
                    session information is ever collected.
                  </p>
                </div>
                <button
                  onClick={handleToggleAnalytics}
                  className={`relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
                    analyticsEnabled ? 'bg-primary-500' : 'bg-stone-600'
                  }`}
                  role="switch"
                  aria-checked={analyticsEnabled}>
                  <span
                    className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
                      analyticsEnabled ? 'translate-x-5' : 'translate-x-0'
                    }`}
                  />
                </button>
              </div>
            </div>
          </div>

          {/* Info Box */}
          <div className="p-4 bg-stone-50 rounded-xl border border-stone-200">
            <div className="flex items-start space-x-3">
              <svg
                className="w-5 h-5 text-stone-400 mt-0.5 flex-shrink-0"
                fill="currentColor"
                viewBox="0 0 20 20">
                <path
                  fillRule="evenodd"
                  d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7-4a1 1 0 11-2 0 1 1 0 012 0zM9 9a1 1 0 000 2v3a1 1 0 001 1h1a1 1 0 100-2v-3a1 1 0 00-1-1H9z"
                  clipRule="evenodd"
                />
              </svg>
              <div>
                <p className="text-xs text-stone-500 leading-relaxed">
                  All analytics and bug reports are fully anonymized. When enabled, we collect only
                  crash information, device type, and the file location of errors. We never access
                  your messages, session data, wallet keys, API keys, or any personally identifiable
                  information. You can change this setting at any time.
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
