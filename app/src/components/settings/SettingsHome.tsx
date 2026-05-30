import type { ReactNode } from 'react';

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
        // Alerts (inbox) + Notifications (preferences) now live together under
        // the Advanced → Notifications hub (see DeveloperOptionsPanel).
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
          id: 'agents-settings',
          title: t('settings.agentsSection.title'),
          description: t('settings.agentsSection.menuDesc'),
          icon: (
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 7h10a2 2 0 012 2v6a2 2 0 01-2 2H7a2 2 0 01-2-2V9a2 2 0 012-2zm2 4h.01M15 11h.01M9.5 15h5"
              />
            </svg>
          ),
          onClick: () => navigateToSettings('agents-settings'),
        },
        {
          id: 'crypto',
          title: t('settings.cryptoSection.title'),
          description: t('settings.cryptoSection.menuDesc'),
          icon: (
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M12 8c-1.657 0-3 .895-3 2s1.343 2 3 2 3 .895 3 2-1.343 2-3 2m0-8c1.11 0 2.08.402 2.599 1M12 8V6m0 10c-1.11 0-2.08-.402-2.599-1M12 16v2m0-12a9 9 0 100 18 9 9 0 000-18z"
              />
            </svg>
          ),
          onClick: () => navigateToSettings('crypto'),
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
