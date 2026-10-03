import { useLocation, useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import CostDashboardPanel from '../../dashboard/CostDashboardPanel';
import UsageLogPanel from '../../dashboard/UsageLogPanel';
import SettingsTabbedPage from '../layout/SettingsTabbedPage';
import BackgroundLoopControls from './ai/BackgroundLoopControls';
import TokenUsagePanel from './TokenUsagePanel';

type TabId = 'costs' | 'log' | 'tokens' | 'background';

const TAB_HASH: Record<TabId, string> = {
  costs: '',
  log: '#log',
  tokens: '#tokens',
  background: '#background',
};

const hashToTab = (hash: string): TabId => {
  if (hash === '#background') return 'background';
  if (hash === '#tokens') return 'tokens';
  if (hash === '#log') return 'log';
  return 'costs';
};

/**
 * Connections → Usage. One page, four views as header chip tabs (the same
 * shell as Connections → LLM): the cost dashboard, the per-call usage log,
 * Tokenjuice token savings, and background loops + the credit ledger. The
 * active tab is reflected in the URL hash (`#log` / `#tokens` / `#background`)
 * so deep links and the legacy ledger-usage/token-usage redirects
 * land on the right view.
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
    <SettingsTabbedPage<TabId>
      title={t('settings.usage.title')}
      description={t('settings.usage.menuDesc')}
      tabsAriaLabel={t('settings.usage.title')}
      tabsTestIdPrefix="usage-tab"
      // The log tab is one table card that fills the body and scrolls its own
      // rows, so the page must not scroll around it.
      scrollable={tab !== 'log'}
      value={tab}
      onChange={selectTab}
      tabs={[
        { id: 'costs', label: t('settings.costDashboard.title') },
        { id: 'log', label: t('settings.costDashboard.usageLog') },
        { id: 'tokens', label: t('settings.tokenUsage.title') },
        { id: 'background', label: t('settings.ai.backgroundLoops') },
      ]}>
      {tab === 'costs' && <CostDashboardPanel embedded />}
      {tab === 'log' && <UsageLogPanel />}
      {tab === 'tokens' && <TokenUsagePanel embedded />}
      {tab === 'background' && <BackgroundActivityTab />}
    </SettingsTabbedPage>
  );
};

/** Background-activity tab body: the loop map plus the credit ledger. */
const BackgroundActivityTab = () => (
  <div className="space-y-4" data-testid="usage-background-tab">
    <BackgroundLoopControls view="all" hideHeader />
  </div>
);

export default UsagePanel;
