import { useLocation, useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import NotificationRoutingPanel from './NotificationRoutingPanel';
import NotificationsPanel from './NotificationsPanel';

type TabId = 'preferences' | 'routing';

const TAB_HASH: Record<TabId, string> = { preferences: '', routing: '#routing' };

const hashToTab = (hash: string): TabId => (hash === '#routing' ? 'routing' : 'preferences');

/**
 * Single Settings entry for notifications. Combines the user-facing
 * preferences (NotificationsPanel) and the routing/intelligence pipeline
 * controls (NotificationRoutingPanel) as two tabs under one header. The
 * active tab is reflected in the URL hash (`#routing`) so deep links from
 * Developer Options still land on the right view.
 */
const NotificationsTabbedPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const location = useLocation();
  const navigate = useNavigate();
  // The router is the single source of truth for the active tab — hash is the
  // only signal needed, so derive directly instead of mirroring it in state.
  const tab: TabId = hashToTab(location.hash);

  const selectTab = (next: TabId) => {
    navigate(`${location.pathname}${TAB_HASH[next]}`, { replace: true });
  };

  const tabs: { id: TabId; label: string }[] = [
    { id: 'preferences', label: t('settings.notifications.tabs.preferences') },
    { id: 'routing', label: t('settings.notifications.tabs.routing') },
  ];

  return (
    <div>
      <SettingsHeader
        title={t('settings.notifications')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div
        role="tablist"
        aria-label={t('settings.notifications')}
        className="flex gap-1 px-4 pt-3 border-b border-stone-200 dark:border-neutral-800">
        {tabs.map(({ id, label }) => {
          const selected = tab === id;
          return (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={selected}
              onClick={() => selectTab(id)}
              className={`px-3 py-2 text-sm font-medium border-b-2 -mb-px transition-colors ${
                selected
                  ? 'border-primary-500 text-stone-900 dark:text-neutral-100'
                  : 'border-transparent text-stone-500 dark:text-neutral-400 hover:text-stone-700 dark:hover:text-neutral-200'
              }`}>
              {label}
            </button>
          );
        })}
      </div>

      {tab === 'preferences' ? (
        <NotificationsPanel embedded />
      ) : (
        <NotificationRoutingPanel embedded />
      )}
    </div>
  );
};

export default NotificationsTabbedPanel;
