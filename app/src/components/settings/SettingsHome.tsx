import { ReactNode, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import type { Locale } from '../../lib/i18n/types';
import { useCoreState } from '../../providers/CoreStateProvider';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import { setLocale } from '../../store/localeSlice';
import { clearAllAppData } from '../../utils/clearAllAppData';
import { BILLING_DASHBOARD_URL } from '../../utils/links';
import { openUrl } from '../../utils/openUrl';
import { resetWalkthrough } from '../walkthrough/AppWalkthrough';
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

// Subtle uppercase section header label separating settings groups
const SectionHeader = ({ label }: { label: string }) => (
  <div className="px-4 pt-5 pb-1">
    <span className="text-[10px] font-semibold tracking-widest uppercase text-stone-400">
      {label}
    </span>
  </div>
);

const SettingsHome = () => {
  const navigate = useNavigate();
  const { navigateToSettings } = useSettingsNavigation();
  const { clearSession, snapshot } = useCoreState();
  const { t } = useT();
  const dispatch = useAppDispatch();
  const currentLocale = useAppSelector(state => state.locale.current);
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
          rightElement: (
            <select
              value={currentLocale}
              onChange={e => dispatch(setLocale(e.target.value as Locale))}
              aria-label={t('settings.language')}
              className="text-sm border border-stone-300 rounded-lg px-3 py-1.5 bg-white text-stone-700 focus:outline-none focus:ring-2 focus:ring-ocean-500/30 cursor-pointer">
              <option value="en">English</option>
              <option value="zh-CN">简体中文</option>
            </select>
          ),
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
    {
      label: t('settings.featuresAndAI'),
      items: [
        {
          id: 'features',
          title: t('settings.features'),
          description: t('settings.featuresDesc'),
          icon: (
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M13 10V3L4 14h7v7l9-11h-7z"
              />
            </svg>
          ),
          onClick: () => navigateToSettings('features'),
        },
        {
          id: 'ai',
          title: t('settings.ai'),
          description: t('settings.aiDesc'),
          icon: (
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z"
              />
            </svg>
          ),
          onClick: () => navigateToSettings('ai'),
        },
      ],
    },
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
        {
          id: 'rewards',
          title: t('settings.rewards'),
          description: t('settings.rewardsDesc'),
          icon: (
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M12 8v13m0-13V6a2 2 0 112 2h-2zm0 0V5.5A2.5 2.5 0 109.5 8H12zm-7 4h14M5 12a2 2 0 110-4h14a2 2 0 110 4M5 12v7a2 2 0 002 2h10a2 2 0 002-2v-7"
              />
            </svg>
          ),
          onClick: () => navigate('/rewards'),
        },
      ],
    },
    {
      label: t('settings.support'),
      items: [
        {
          id: 'restart-tour',
          title: t('settings.restartTour'),
          description: t('settings.restartTourDesc'),
          icon: (
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
              />
            </svg>
          ),
          onClick: () => {
            resetWalkthrough();
            navigate('/home');
          },
        },
        {
          id: 'about',
          title: t('settings.about'),
          description: t('settings.aboutDesc'),
          icon: (
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
              />
            </svg>
          ),
          onClick: () => navigateToSettings('about'),
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
        {/* Grouped sections with section headers */}
        {settingsSections.map(section => (
          <div key={section.label}>
            <SectionHeader label={section.label} />
            {section.items.map((item, index) => (
              <SettingsMenuItem
                key={item.id}
                icon={item.icon}
                title={item.title}
                description={item.description}
                onClick={item.onClick}
                dangerous={item.dangerous}
                isFirst={index === 0}
                isLast={index === section.items.length - 1}
                rightElement={item.rightElement}
              />
            ))}
          </div>
        ))}

        {/* Danger Zone */}
        <SectionHeader label={t('settings.dangerZone')} />
        {destructiveItems.map((item, index) => (
          <SettingsMenuItem
            key={item.id}
            icon={item.icon}
            title={item.title}
            description={item.description}
            onClick={item.onClick}
            dangerous={item.dangerous}
            isFirst={index === 0}
            isLast={index === destructiveItems.length - 1}
          />
        ))}
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
