import { useEffect, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { type AISettings, loadAISettings } from '../../../services/api/aiSettingsApi';
import CostDashboardPanel from '../../dashboard/CostDashboardPanel';
import SettingsHeader from '../components/SettingsHeader';
import { SettingsStatusLine } from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import { BackgroundLoopControls } from './AIPanel';

type TabId = 'costs' | 'background';

const TAB_HASH: Record<TabId, string> = { costs: '', background: '#background' };

const hashToTab = (hash: string): TabId => (hash === '#background' ? 'background' : 'costs');

/**
 * Single Settings entry for usage & limits. Combines the cost dashboard
 * (charts, budgets, usage log) and the background-activity controls
 * (heartbeat cadences + usage ledger, previously the separate Heartbeat and
 * Usage-ledger pages) as two tabs under one header. The active tab is
 * reflected in the URL hash (`#background`) so deep links and the legacy
 * heartbeat/ledger-usage redirects land on the right view.
 */
const UsagePanel = () => {
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
    { id: 'costs', label: t('settings.costDashboard.title') },
    { id: 'background', label: t('settings.heartbeat.title') },
  ];

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title={t('settings.usage.title')}
        showBackButton
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div
        role="tablist"
        aria-label={t('settings.usage.title')}
        className="flex gap-1 px-4 pt-3 border-b border-neutral-200 dark:border-neutral-800">
        {tabs.map(({ id, label }) => {
          const selected = tab === id;
          return (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={selected}
              data-testid={`usage-tab-${id}`}
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

      {tab === 'costs' ? <CostDashboardPanel embedded /> : <BackgroundActivityTab />}
    </div>
  );
};

/**
 * Background-activity tab body. Fetches the AI settings snapshot (routing map
 * + cloud providers) that BackgroundLoopControls needs — lazily, only when
 * this tab is mounted, so the default Costs tab doesn't pay for it.
 */
const BackgroundActivityTab = () => {
  const { t } = useT();
  const [snapshot, setSnapshot] = useState<AISettings | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    loadAISettings()
      .then(s => {
        if (!cancelled) setSnapshot(s);
      })
      .catch(err => {
        if (!cancelled) setLoadError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div className="p-4 space-y-3" data-testid="usage-background-tab">
      <SettingsStatusLine saving={false} error={loadError} savingLabel="" />
      {snapshot ? (
        <BackgroundLoopControls
          view="all"
          hideHeader
          routing={snapshot.routing}
          cloudProviders={snapshot.cloudProviders}
        />
      ) : !loadError ? (
        <div className="text-xs text-neutral-500 dark:text-neutral-400">{t('common.loading')}</div>
      ) : null}
    </div>
  );
};

export default UsagePanel;
