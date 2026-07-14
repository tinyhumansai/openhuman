import { useEffect, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { type AISettings, loadAISettings } from '../../../services/api/aiSettingsApi';
import CostDashboardPanel from '../../dashboard/CostDashboardPanel';
import { SettingsStatusLine } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';
import { BackgroundLoopControls } from './AIPanel';
import TokenUsagePanel from './TokenUsagePanel';

type TabId = 'costs' | 'tokens' | 'background';

const TAB_HASH: Record<TabId, string> = { costs: '', tokens: '#tokens', background: '#background' };

const hashToTab = (hash: string): TabId => {
  if (hash === '#background') return 'background';
  if (hash === '#tokens') return 'tokens';
  return 'costs';
};

/**
 * Single Settings entry for usage & limits. Combines the cost dashboard
 * (charts, budgets, usage log), the Tokenjuice token-savings surface, and the
 * background-activity controls (heartbeat cadences + usage ledger, previously
 * the separate Heartbeat and Usage-ledger pages) as tabs under one header. The
 * active tab is reflected in the URL hash (`#tokens` / `#background`) so deep
 * links and the legacy heartbeat/ledger-usage/token-usage redirects land on
 * the right view.
 */
const UsagePanel = () => {
  const { t } = useT();
  const location = useLocation();
  const navigate = useNavigate();
  // The router is the single source of truth for the active tab.
  const tab: TabId = hashToTab(location.hash);

  const selectTab = (next: TabId) => {
    navigate(`${location.pathname}${location.search}${TAB_HASH[next]}`, { replace: true });
  };

  return (
    <SettingsPanel<TabId>
      description={t('settings.usage.menuDesc')}
      tabsAriaLabel={t('settings.usage.title')}
      tabsTestIdPrefix="usage-tab"
      value={tab}
      onChange={selectTab}
      tabs={[
        {
          id: 'costs',
          label: t('settings.costDashboard.title'),
          content: <CostDashboardPanel embedded />,
        },
        {
          id: 'tokens',
          label: t('settings.tokenUsage.title'),
          content: <TokenUsagePanel embedded />,
        },
        {
          id: 'background',
          label: t('settings.heartbeat.title'),
          content: <BackgroundActivityTab />,
        },
      ]}
    />
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
    <div className="p-4 space-y-5" data-testid="usage-background-tab">
      <SettingsStatusLine saving={false} error={loadError} savingLabel="" />
      {snapshot ? (
        <BackgroundLoopControls
          view="all"
          hideHeader
          routing={snapshot.routing}
          cloudProviders={snapshot.cloudProviders}
        />
      ) : !loadError ? (
        <div className="text-xs text-content-muted">{t('common.loading')}</div>
      ) : null}
    </div>
  );
};

export default UsagePanel;
