import type { ReactNode } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import { useCoreState } from '../../providers/CoreStateProvider';
import { BILLING_DASHBOARD_URL } from '../../utils/links';
import { isLocalSessionToken } from '../../utils/localSession';
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
  const { t } = useT();
  const { snapshot } = useCoreState();
  const isLocalSession = isLocalSessionToken(snapshot.sessionToken);

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
          id: 'devices',
          title: 'Devices',
          description: 'Pair iOS phones with this OpenHuman',
          icon: (
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M12 18h.01M8 21h8a2 2 0 002-2V5a2 2 0 00-2-2H8a2 2 0 00-2 2v14a2 2 0 002 2z"
              />
            </svg>
          ),
          onClick: () => navigateToSettings('devices'),
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
          id: 'appearance',
          title: t('settings.appearance.title'),
          description: t('settings.appearance.menuDesc'),
          icon: (
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M21 12.79A9 9 0 1111.21 3 7 7 0 0021 12.79z"
              />
            </svg>
          ),
          onClick: () => navigateToSettings('appearance'),
        },
        {
          id: 'mascot',
          title: t('settings.mascot.menuTitle'),
          description: t('settings.mascot.menuDesc'),
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
    // Billing & Rewards requires a backend-authenticated session.
    // Hidden in local/offline mode — no auth headers are sent and the
    // billing dashboard would not recognise the session.
    ...(!isLocalSession
      ? [
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
          } satisfies SettingsSection,
        ]
      : []),
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

  // Log Out and Clear App Data now live on the Account page (Settings → Account)
  // alongside the recovery phrase, team, privacy, and migration entries.

  return (
    <div className="z-10 relative">
      <div data-walkthrough="settings-menu">
        <SettingsHeader />
      </div>

      <div>
        {/* Flat list — group titles removed for clarity. Destructive
            actions (Log Out, Clear App Data) now live on the Account page. */}
        {(() => {
          const flatItems = settingsSections.flatMap(s => s.items);
          return flatItems.map((item, index) => (
            <SettingsMenuItem
              key={item.id}
              icon={item.icon}
              title={item.title}
              description={item.description}
              onClick={item.onClick}
              testId={`settings-nav-${item.id}`}
              dangerous={item.dangerous}
              isFirst={index === 0}
              isLast={index === flatItems.length - 1}
              rightElement={item.rightElement}
            />
          ));
        })()}
      </div>
    </div>
  );
};

export default SettingsHome;
