import { useLocation, useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import Webhooks from '../../../pages/Webhooks';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import ComposioPanel from './ComposioPanel';
import TaskSourcesPanel from './TaskSourcesPanel';

type TabId = 'task-sources' | 'composio' | 'webhooks';

const TAB_HASH: Record<TabId, string> = {
  'task-sources': '',
  composio: '#composio',
  webhooks: '#webhooks',
};

const hashToTab = (hash: string): TabId => {
  if (hash === '#composio') return 'composio';
  if (hash === '#webhooks') return 'webhooks';
  return 'task-sources';
};

/**
 * Single Settings entry for integrations. Combines the task-source toggles
 * (TaskSourcesPanel), the Composio routing/auth controls (ComposioPanel) and
 * the webhook trigger history/triage (Webhooks page) as three tabs under one
 * header. The active tab is reflected in the URL hash (`#composio`,
 * `#webhooks`) so deep links and the legacy task-sources/composio-routing/
 * webhooks-triggers redirects land on the right view.
 */
const IntegrationsPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const location = useLocation();
  const navigate = useNavigate();
  // The router is the single source of truth for the active tab.
  const tab: TabId = hashToTab(location.hash);

  const selectTab = (next: TabId) => {
    navigate(`${location.pathname}${TAB_HASH[next]}`, { replace: true });
  };

  const tabs: { id: TabId; label: string }[] = [
    { id: 'task-sources', label: t('settings.taskSources.title') },
    { id: 'composio', label: t('settings.developerMenu.composioRouting.title') },
    { id: 'webhooks', label: t('settings.developerMenu.composeioTriggers.title') },
  ];

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title={t('settings.integrations.title')}
        showBackButton
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div
        role="tablist"
        aria-label={t('settings.integrations.title')}
        className="flex gap-1 px-4 pt-3 border-b border-neutral-200 dark:border-neutral-800">
        {tabs.map(({ id, label }) => {
          const selected = tab === id;
          return (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={selected}
              data-testid={`integrations-tab-${id}`}
              onClick={() => selectTab(id)}
              className={`px-3 py-2 text-sm font-medium border-b-2 -mb-px transition-colors ${
                selected
                  ? 'border-primary-500 text-neutral-800 dark:text-neutral-100'
                  : 'border-transparent text-neutral-500 dark:text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200'
              }`}>
              {label}
            </button>
          );
        })}
      </div>

      {tab === 'task-sources' && <TaskSourcesPanel embedded />}
      {tab === 'composio' && (
        <div className="p-4">
          <ComposioPanel embedded />
        </div>
      )}
      {tab === 'webhooks' && <Webhooks embedded />}
    </div>
  );
};

export default IntegrationsPanel;
