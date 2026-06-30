import { useLocation, useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import SettingsPanel from '../layout/SettingsPanel';
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
  const location = useLocation();
  const navigate = useNavigate();
  // The router is the single source of truth for the active tab — hash is the
  // only signal needed, so derive directly instead of mirroring it in state.
  const tab: TabId = hashToTab(location.hash);

  const selectTab = (next: TabId) => {
    navigate(`${location.pathname}${TAB_HASH[next]}`, { replace: true });
  };

  return (
    <SettingsPanel<TabId>
      description={t('settings.notifications.menuDesc')}
      tabsAriaLabel={t('settings.notifications')}
      value={tab}
      onChange={selectTab}
      tabs={[
        {
          id: 'preferences',
          label: t('settings.notifications.tabs.preferences'),
          content: <NotificationsPanel embedded />,
        },
        {
          id: 'routing',
          label: t('settings.notifications.tabs.routing'),
          content: <NotificationRoutingPanel embedded />,
        },
      ]}
    />
  );
};

export default NotificationsTabbedPanel;
