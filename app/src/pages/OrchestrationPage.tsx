/**
 * OrchestrationPage — the TinyPlace multi-agent orchestration surface.
 *
 * Promoted out of Brain into a first-class sidebar destination (`/orchestration`),
 * it now fans out into four sub-pages projected into the shell's dynamic sidebar
 * region (like Brain), driven by `?tab=`:
 *
 *   - **agent**       — chat with the main agent + its subconscious steering loop
 *   - **connections** — manage the agent's accepted peer connections
 *   - **discover**    — grow the network: own discoverability, link + inbound requests
 *   - **usage**       — stats: connections, credit balance, spend, token savings
 */
import { useCallback, useMemo } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import PanelPage from '../components/layout/PanelPage';
import { SidebarContent } from '../components/layout/shell/SidebarSlot';
import TwoPaneNav from '../components/layout/TwoPaneNav';
import AgentChatPanel from '../components/orchestration/AgentChatPanel';
import ConnectionsPanel from '../components/orchestration/ConnectionsPanel';
import DiscoverPanel from '../components/orchestration/DiscoverPanel';
import UsagePanel from '../components/orchestration/UsagePanel';
import { useT } from '../lib/i18n/I18nContext';

type OrchestrationTab = 'agent' | 'connections' | 'discover' | 'usage';

const ORCHESTRATION_TABS: readonly OrchestrationTab[] = [
  'agent',
  'connections',
  'discover',
  'usage',
];

/** Small inline icon helper for the Orchestration sub-nav (matches Brain). */
const navIcon = (d: string) => (
  <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d={d} />
  </svg>
);

export default function OrchestrationPage() {
  const { t } = useT();
  const location = useLocation();
  const navigate = useNavigate();

  const activeTab = useMemo<OrchestrationTab>(() => {
    const raw = new URLSearchParams(location.search).get('tab');
    return (ORCHESTRATION_TABS as readonly string[]).includes(raw ?? '')
      ? (raw as OrchestrationTab)
      : 'agent';
  }, [location.search]);

  const setActiveTab = useCallback(
    (tab: OrchestrationTab) => {
      const params = new URLSearchParams(location.search);
      params.set('tab', tab);
      navigate({ pathname: location.pathname, search: `?${params.toString()}` });
    },
    [location.pathname, location.search, navigate]
  );

  console.debug('[orchestration] page mount tab=%s', activeTab);

  return (
    <div className="h-full">
      <SidebarContent>
        <div className="h-full overflow-hidden">
          <TwoPaneNav
            ariaLabel={t('nav.orchestration')}
            selected={activeTab}
            onSelect={value => setActiveTab(value as OrchestrationTab)}
            groups={[
              {
                label: t('orchPage.group.agent'),
                items: [
                  {
                    value: 'agent',
                    label: t('orchPage.agent.nav'),
                    icon: navIcon(
                      'M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z'
                    ),
                  },
                ],
              },
              {
                label: t('orchPage.group.network'),
                items: [
                  {
                    value: 'connections',
                    label: t('orchPage.connections.nav'),
                    icon: navIcon('M13 10V3L4 14h7v7l9-11h-7z M17 8a3 3 0 100-6 3 3 0 000 6z'),
                  },
                  {
                    value: 'discover',
                    label: t('orchPage.discover.nav'),
                    icon: navIcon('M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z'),
                  },
                ],
              },
              {
                label: t('orchPage.group.insights'),
                items: [
                  {
                    value: 'usage',
                    label: t('orchPage.usage.nav'),
                    icon: navIcon(
                      'M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z'
                    ),
                  },
                ],
              },
            ]}
            header={
              <p className="min-w-0 text-[11px] text-content-muted">{t('orchPage.subtitle')}</p>
            }
          />
        </div>
      </SidebarContent>

      <div className="mx-auto h-full w-full max-w-5xl">
        {activeTab === 'agent' ? (
          // The chat panel manages its own full-height layout + scroll.
          <div className="h-full p-4">
            <AgentChatPanel />
          </div>
        ) : (
          <PanelPage contentClassName="p-4">
            <div className="mx-auto max-w-3xl animate-fade-up">
              {activeTab === 'connections' && (
                <ConnectionsPanel onDiscover={() => setActiveTab('discover')} />
              )}
              {activeTab === 'discover' && <DiscoverPanel />}
              {activeTab === 'usage' && <UsagePanel />}
            </div>
          </PanelPage>
        )}
      </div>
    </div>
  );
}
