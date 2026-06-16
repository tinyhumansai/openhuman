import { Navigate, useLocation, useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import Webhooks from '../../../pages/Webhooks';
import PanelPage from '../../layout/PanelPage';
import SettingsBackButton from '../components/SettingsBackButton';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import TaskSourcesPanel from './TaskSourcesPanel';

type TabId = 'task-sources' | 'webhooks';

const TAB_HASH: Record<TabId, string> = { 'task-sources': '', webhooks: '#webhooks' };

const hashToTab = (hash: string): TabId => {
  if (hash === '#webhooks') return 'webhooks';
  return 'task-sources';
};

/**
 * Single Settings entry for integrations. Combines the task-source toggles
 * (TaskSourcesPanel) and the webhook trigger history/triage (Webhooks page) as
 * two tabs under one header. The active tab is reflected in the URL hash
 * (`#webhooks`) so deep links and the legacy redirects land on the right view.
 * Composio (API key + routing) moved to Connections → API keys.
 */
const IntegrationsPanel = () => {
  const { t } = useT();
  const { navigateBack } = useSettingsNavigation();
  const location = useLocation();
  const navigate = useNavigate();

  // Legacy deep link: the old Composio tab lived at `#composio` on this page.
  // It now lives under Connections → API keys, so normalize the bookmark.
  if (location.hash === '#composio') {
    return <Navigate to="/connections?tab=composio-key" replace />;
  }

  // The router is the single source of truth for the active tab.
  const tab: TabId = hashToTab(location.hash);

  const selectTab = (next: TabId) => {
    navigate(`${location.pathname}${TAB_HASH[next]}`, { replace: true });
  };

  return (
    <PanelPage<TabId>
      className="z-10"
      description={t('settings.integrations.menuDesc')}
      leading={<SettingsBackButton onBack={navigateBack} />}
      tabsAriaLabel={t('settings.integrations.title')}
      tabsTestIdPrefix="integrations-tab"
      value={tab}
      onChange={selectTab}
      tabs={[
        {
          id: 'task-sources',
          label: t('settings.taskSources.title'),
          content: <TaskSourcesPanel embedded />,
        },
        {
          id: 'webhooks',
          label: t('settings.developerMenu.composeioTriggers.title'),
          content: <Webhooks embedded />,
        },
      ]}
    />
  );
};

export default IntegrationsPanel;
