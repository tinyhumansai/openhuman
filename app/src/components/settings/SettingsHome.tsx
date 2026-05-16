import { ReactNode, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import { useCoreState } from '../../providers/CoreStateProvider';
import { clearAllAppData } from '../../utils/clearAllAppData';
import { BILLING_DASHBOARD_URL } from '../../utils/links';
import { openUrl } from '../../utils/openUrl';
import LanguageSelect from '../LanguageSelect';
import SettingsHeader from './components/SettingsHeader';
import SettingsMenuItem from './components/SettingsMenuItem';
import { useSettingsNavigation } from './hooks/useSettingsNavigation';

interface SettingsSection {
  label: string;
  items: SettingsItem[];
}

interface SettingsItem {
  id: string;
  title: string;
  description: string;
  icon: ReactNode;
  onClick?: () => void;
  dangerous?: boolean;
  rightElement?: ReactNode;
}

const SettingsHome = () => {
  const navigate = useNavigate();
  const { navigateToSettings } = useSettingsNavigation();
  const { clearSession, snapshot } = useCoreState();
  const { t } = useT();
  const [showLogoutAndClearModal, setShowLogoutAndClearModal] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleLogout = async () => {
    try {
      await clearSession();
    } catch (err) {
      console.warn('[Settings] Rust logout failed:', err);
      setError(t('clearData.failedLogout'));
    }
  };

  const handleLogoutAndClearData = async () => {
    try {
      setIsLoading(true);
      setError(null);
      const currentUserId = snapshot.auth.userId ?? snapshot.currentUser?._id ?? null;
      await clearAllAppData({ clearSession, userId: currentUserId }); // restarts the app
    } catch (_error) {
      setError(t('clearData.failed'));
    } finally {
      setIsLoading(false);
    }
  };

  const settingsSections: SettingsSection[] = [
    {
      label: t('settings.general'),
      items: [
        {
          id: 'account',
          title: t('settings.account'),
          description: t('settings.accountDesc'),
          icon: (
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z"
              />
            </svg>
          ),
          onClick: () => navigateToSettings('account'),
        },
        {
          id: 'alerts',
          title: t('nav.alerts'),
          description: t('settings.alertsDesc'),
          icon: (
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9"
              />
            </svg>
          ),
          onClick: () => navigate('/notifications'),
        },
        {
          id: 'notifications',
          title: t('settings.notifications'),
          description: t('settings.notificationsDesc'),
          icon: (
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9"
              />
            </svg>
          ),
          onClick: () => navigateToSettings('notifications'),
        },
        {
          id: 'language',
          title: t('settings.language'),
          description: t('settings.languageDesc'),
          icon: (
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M3 5h12M9 3v2m1.048 9.5A18.022 18.022 0 016.412 9m6.088 9h7M11 21l5-10 5 10M12.751 5C11.783 10.77 8.07 15.61 3 18.129"
              />
            </svg>
          ),
          rightElement: <LanguageSelect ariaLabel={t('settings.language')} />,
        },
        {
          id: 'mascot',
          title: 'Mascot',
          description: 'Pick the mascot color used across the app',
          icon: (
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M12 21a9 9 0 100-18 9 9 0 000 18zM9 10h.01M15 10h.01M9.5 15c.83.67 1.67 1 2.5 1s1.67-.33 2.5-1"
              />
            </svg>
          ),
          onClick: () => navigateToSettings('mascot'),
        },
      ],
    },
    // Features tile (Screen Awareness / Messaging Channels / Notifications /
    // Tools) used to live here. Everything under it moved into Advanced
    // (DeveloperOptionsPanel), so the section is gone from the home menu.
    {
      label: t('settings.billingAndRewards'),
      items: [
        {
          id: 'billing',
          title: t('settings.billingUsage'),
          description: t('settings.billingUsageDesc'),
          icon: (
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M3 10h18M7 15h1m4 0h1m-7 4h12a3 3 0 003-3V8a3 3 0 00-3-3H5a3 3 0 00-3 3v8a3 3 0 003 3z"
              />
            </svg>
          ),
          onClick: () => {
            openUrl(BILLING_DASHBOARD_URL).catch(() => {});
          },
        },
      ],
    },
    {
      label: t('settings.advanced'),
      items: [
        {
          id: 'developer-options',
          title: t('settings.developerOptions'),
          description: t('settings.developerOptionsDesc'),
          icon: (
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4"
              />
            </svg>
          ),
          onClick: () => navigateToSettings('developer-options'),
        },
      ],
    },
  ];

  // Destructive actions — rendered separately under "Danger Zone" heading
  const destructiveItems: SettingsItem[] = [
    {
      id: 'logout-and-clear',
      title: t('settings.clearAppData'),
      description: t('settings.clearAppDataDesc'),
      icon: (
        <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1"
          />
        </svg>
      ),
      onClick: () => setShowLogoutAndClearModal(true),
      dangerous: true,
    },
    {
      id: 'logout',
      title: t('settings.logOut'),
      description: t('settings.logOutDesc'),
      icon: (
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1"
          />
        </svg>
      ),
      onClick: handleLogout,
      dangerous: true,
    },
  ];

  return (
    <div className="z-10 relative">
      <div data-walkthrough="settings-menu">
        <SettingsHeader />
      </div>

      <div>
        {/* Flat list — group titles removed for clarity. Regular items first,
            destructive items appended at the end. */}
        {(() => {
          const flatItems = settingsSections.flatMap(s => s.items).concat(destructiveItems);
          return flatItems.map((item, index) => (
            <SettingsMenuItem
              key={item.id}
              icon={item.icon}
              title={item.title}
              description={item.description}
              onClick={item.onClick}
              dangerous={item.dangerous}
              isFirst={index === 0}
              isLast={index === flatItems.length - 1}
              rightElement={item.rightElement}
            />
          ));
        })()}
      </div>

      {/* Log Out & Clear Data Confirmation Modal */}
      {showLogoutAndClearModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/30">
          <div className="bg-white rounded-2xl max-w-md w-full p-6 border border-stone-200">
            <div className="flex items-center gap-3 mb-4">
              <div className="w-10 h-10 rounded-lg bg-amber-100 flex items-center justify-center">
                <svg
                  className="w-5 h-5 text-amber-400"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1"
                  />
                </svg>
              </div>
              <div>
                <h3 className="text-lg font-semibold text-stone-900">{t('clearData.title')}</h3>
              </div>
            </div>

            <div className="mb-6">
              <div className="text-stone-700 text-sm leading-relaxed">
                <p>{t('clearData.warning')}</p>
                <ul className="list-disc pl-5 mt-2 space-y-1">
                  <li>{t('clearData.bulletSettings')}</li>
                  <li>{t('clearData.bulletCache')}</li>
                  <li>{t('clearData.bulletWorkspace')}</li>
                  <li>{t('clearData.bulletOther')}</li>
                </ul>
                <p className="mt-3">{t('clearData.irreversible')}</p>
              </div>

              {error && (
                <div className="mt-3 p-3 rounded-lg bg-coral-100 border border-coral-500/20">
                  <p className="text-coral-600 text-sm">{error}</p>
                </div>
              )}
            </div>

            <div className="flex gap-3">
              <button
                onClick={() => {
                  setShowLogoutAndClearModal(false);
                  setError(null);
                }}
                disabled={isLoading}
                className="flex-1 px-4 py-2 rounded-lg border border-stone-200 text-stone-700 hover:bg-stone-100 transition-colors disabled:opacity-50">
                {t('common.cancel')}
              </button>
              <button
                onClick={handleLogoutAndClearData}
                disabled={isLoading}
                className="flex-1 px-4 py-2 rounded-sm bg-amber-600 hover:bg-amber-500 text-white transition-colors disabled:opacity-50 flex items-center justify-center gap-2">
                {isLoading && (
                  <svg className="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                    <circle
                      className="opacity-25"
                      cx="12"
                      cy="12"
                      r="10"
                      stroke="currentColor"
                      strokeWidth="4"
                    />
                    <path
                      className="opacity-75"
                      fill="currentColor"
                      d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                    />
                  </svg>
                )}
                {isLoading ? t('clearData.clearing') : t('clearData.title')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

export default SettingsHome;
