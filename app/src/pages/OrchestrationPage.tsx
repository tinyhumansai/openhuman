/**
 * OrchestrationPage — the TinyPlace multi-agent orchestration surface.
 *
 * Promoted out of Brain into a first-class sidebar destination (`/orchestration`),
 * it splits into two sidebar destinations projected into the shell's dynamic
 * sidebar region, driven by `?tab=`:
 *
 *   - **agent**   — chat with the main agent + its subconscious steering loop
 *   - **network** — one page with a chip sub-nav (`?sub=`) over the peer-network
 *                   views: **connections**, **discover**, **usage**
 *
 * The sidebar lists the two destinations flat (no category headers), then a
 * separator, then a live list of the agent's active peer sessions — mirroring
 * how the chat window lists active threads. Clicking a session opens it in the
 * agent chat (via the `session` query param, read by {@link AgentChatPanel}).
 */
import { useCallback, useMemo } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import ChipTabs from '../components/layout/ChipTabs';
import PageSectionHeader from '../components/layout/PageSectionHeader';
import PanelPage from '../components/layout/PanelPage';
import { SidebarContent } from '../components/layout/shell/SidebarSlot';
import TwoPaneNav from '../components/layout/TwoPaneNav';
import ActiveSubagentsRail from '../components/orchestration/ActiveSubagentsRail';
import AgentChatPanel from '../components/orchestration/AgentChatPanel';
import ConnectionsPanel from '../components/orchestration/ConnectionsPanel';
import MedullaDemoChat from '../components/orchestration/demo/MedullaDemoChat';
import MedullaDemoGraph from '../components/orchestration/demo/MedullaDemoGraph';
import MedullaDemoNetwork from '../components/orchestration/demo/MedullaDemoNetwork';
import DiscoverPanel from '../components/orchestration/DiscoverPanel';
import MedullaOverviewPanel from '../components/orchestration/MedullaOverviewPanel';
import OrchestratorTaskBoard from '../components/orchestration/OrchestratorTaskBoard';
import OverviewPanel from '../components/orchestration/OverviewPanel';
import UsagePanel from '../components/orchestration/UsagePanel';
import { useT } from '../lib/i18n/I18nContext';
import { useMedullaAccess } from '../lib/orchestration/useMedullaAccess';
import { useContactSessions } from '../lib/orchestration/useOrchestrationSessions';

type OrchestrationTab = 'medulla' | 'overview' | 'agent' | 'tasks' | 'network';
type NetworkSub = 'connections' | 'discover' | 'usage';

const NETWORK_SUBS: readonly NetworkSub[] = ['connections', 'discover', 'usage'];

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
  const contactSessions = useContactSessions();
  // Without Medulla access, Medulla-specific live surfaces are replaced by a
  // scale showcase. The global task board remains available to every user.
  const hasMedullaAccess = useMedullaAccess();

  const params = useMemo(() => new URLSearchParams(location.search), [location.search]);
  const rawTab = params.get('tab');
  const rawSub = params.get('sub');

  // `?tab=connections|discover|usage` is a legacy deep link → the network page
  // with that sub selected. New links use `?tab=network&sub=…`.
  // Default landing is the Medulla overview (the orchestration overview page).
  const activeTab: OrchestrationTab =
    rawTab === 'overview'
      ? 'overview'
      : rawTab === 'agent'
        ? 'agent'
        : rawTab === 'tasks'
          ? 'tasks'
          : rawTab === 'network' || NETWORK_SUBS.includes(rawTab as NetworkSub)
            ? 'network'
            : 'medulla';

  const networkSub: NetworkSub = NETWORK_SUBS.includes(rawTab as NetworkSub)
    ? (rawTab as NetworkSub)
    : NETWORK_SUBS.includes(rawSub as NetworkSub)
      ? (rawSub as NetworkSub)
      : 'connections';

  const openSessionId = params.get('session');

  const updateParams = useCallback(
    (mut: (p: URLSearchParams) => void) => {
      const next = new URLSearchParams(location.search);
      mut(next);
      navigate({ pathname: location.pathname, search: `?${next.toString()}` });
    },
    [location.pathname, location.search, navigate]
  );

  const setActiveTab = useCallback(
    (tab: OrchestrationTab) => {
      updateParams(p => {
        p.set('tab', tab);
        // Selecting Chat returns to the master chat — drop any open session so
        // the session subpage closes (there's no in-view back button).
        if (tab === 'agent') p.delete('session');
        // Landing on the network page needs a valid sub selected.
        if (tab === 'network' && !NETWORK_SUBS.includes(p.get('sub') as NetworkSub)) {
          p.set('sub', 'connections');
        }
      });
    },
    [updateParams]
  );

  const setNetworkSub = useCallback(
    (sub: NetworkSub) => {
      updateParams(p => {
        p.set('tab', 'network');
        p.set('sub', sub);
      });
    },
    [updateParams]
  );

  // Open (or close, when null) a peer session in the agent chat. Always lands on
  // the agent tab so the session's transcript is actually visible.
  const setOpenSessionId = useCallback(
    (sessionId: string | null) => {
      updateParams(p => {
        p.set('tab', 'agent');
        if (sessionId) p.set('session', sessionId);
        else p.delete('session');
      });
    },
    [updateParams]
  );

  console.debug(
    '[orchestration] page mount tab=%s sub=%s session=%s medullaAccess=%s',
    activeTab,
    networkSub,
    openSessionId,
    hasMedullaAccess
  );

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
                // Flat list — no category headers: Overview · Chat · Agent
                // graph · Tasks · Network. Overview (Medulla) is the landing.
                items: [
                  {
                    value: 'medulla',
                    label: t('orchPage.medulla.nav'),
                    icon: navIcon(
                      'M12 3v2m0 14v2m9-9h-2M5 12H3m14.657-6.657l-1.414 1.414M7.757 16.243l-1.414 1.414m0-11.314l1.414 1.414m8.486 8.486l1.414 1.414M15 12a3 3 0 11-6 0 3 3 0 016 0z'
                    ),
                  },
                  {
                    value: 'agent',
                    label: t('orchPage.agent.nav'),
                    icon: navIcon(
                      'M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z'
                    ),
                  },
                  {
                    value: 'overview',
                    label: t('orchPage.overview.nav'),
                    icon: navIcon(
                      'M4 5a2 2 0 012-2h12a2 2 0 012 2M9 12a2 2 0 11-4 0 2 2 0 014 0zm10 4a2 2 0 11-4 0 2 2 0 014 0zM7 12l7 4'
                    ),
                  },
                  {
                    value: 'tasks',
                    label: t('orchPage.tasks.nav'),
                    icon: navIcon(
                      'M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2m-6 9l2 2 4-4'
                    ),
                  },
                  {
                    value: 'network',
                    label: t('orchPage.group.network'),
                    icon: navIcon('M13 10V3L4 14h7v7l9-11h-7z M17 8a3 3 0 100-6 3 3 0 000 6z'),
                  },
                ],
              },
            ]}
            footer={
              // Active sub-agents, grouped by instance (contact) with a
              // connection-status dot. Clicking a sub-agent opens its chat.
              // Driven by live peer sessions, so it's hidden in showcase mode
              // (no live sub-agents without Medulla access).
              hasMedullaAccess ? (
                <ActiveSubagentsRail
                  byContact={contactSessions.byContact}
                  openSessionId={openSessionId}
                  isAgentTab={activeTab === 'agent'}
                  onOpenSession={setOpenSessionId}
                />
              ) : undefined
            }
          />
        </div>
      </SidebarContent>

      {activeTab === 'medulla' ? (
        // Orchestration overview — the Medulla teaser / early-access landing.
        <MedullaOverviewPanel />
      ) : activeTab === 'overview' ? (
        // Interactive graph of the agent / sub-agent system — or the scale
        // showcase (core → 2 devices → 120 agents) without Medulla access.
        hasMedullaAccess ? (
          <OverviewPanel />
        ) : (
          <MedullaDemoGraph />
        )
      ) : activeTab === 'agent' ? (
        // Full-bleed so it reads exactly like the normal chat page (dark
        // background, floating composer, one vertical scroll) — or a read-only
        // demo conversation (composer disabled) without Medulla access.
        hasMedullaAccess ? (
          <div className="h-full">
            <AgentChatPanel openSessionId={openSessionId} onOpenSession={setOpenSessionId} />
          </div>
        ) : (
          <MedullaDemoChat />
        )
      ) : activeTab === 'tasks' ? (
        // One global Kanban board owned by the orchestrator (not per-thread).
        // Tasks predate Medulla access and must remain usable without it.
        <div className="mx-auto h-full w-full max-w-3xl">
          <PanelPage contentClassName="p-4">
            <div className="animate-fade-up space-y-4">
              <PageSectionHeader
                title={t('orchPage.tasks.nav')}
                description={t('orchPage.tasks.subtitle')}
              />
              <OrchestratorTaskBoard />
            </div>
          </PanelPage>
        </div>
      ) : hasMedullaAccess ? (
        <div className="mx-auto h-full w-full max-w-3xl">
          {/* Network: one page with a Brain-style chip sub-nav (flush pills, no
              header background) over connections/discover/usage, aligned to the
              same content column. */}
          <PanelPage contentClassName="p-4">
            <div className="mx-auto max-w-3xl space-y-5 animate-fade-up">
              <PageSectionHeader
                title={t('orchPage.group.network')}
                description={t('orchPage.network.desc')}
                tabs={
                  <ChipTabs<NetworkSub>
                    as="tab"
                    ariaLabel={t('orchPage.group.network')}
                    testIdPrefix="orch-network"
                    className="inline-flex flex-wrap items-center gap-1.5"
                    items={[
                      { id: 'connections', label: t('orchPage.connections.nav') },
                      { id: 'discover', label: t('orchPage.discover.nav') },
                      { id: 'usage', label: t('orchPage.usage.nav') },
                    ]}
                    value={networkSub}
                    onChange={setNetworkSub}
                  />
                }
              />
              {networkSub === 'connections' && (
                <ConnectionsPanel
                  onDiscover={() => setNetworkSub('discover')}
                  onInitializeAgent={() => setActiveTab('agent')}
                />
              )}
              {networkSub === 'discover' && <DiscoverPanel />}
              {networkSub === 'usage' && <UsagePanel />}
            </div>
          </PanelPage>
        </div>
      ) : (
        // Scale showcase: fake peer-agent mesh with the preview banner.
        <MedullaDemoNetwork />
      )}
    </div>
  );
}
