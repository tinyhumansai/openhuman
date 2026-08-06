/**
 * OrchestrationView — the TinyPlace multi-agent orchestration surface, embedded
 * as the Brain page's **Orchestration** sub-tab (`/brain?tab=orchestration`).
 *
 * It was previously a first-class sidebar destination (`/orchestration`) with
 * its own projected sidebar. Folding it back under Brain means the top-level
 * views — **Overview** (Medulla), **Chat**, **Agent graph**, **Tasks**,
 * **Network** — become an in-content chip row instead of a left rail, so Brain
 * keeps a single sidebar. State travels in query params that don't collide with
 * Brain's own `?tab=`:
 *
 *   - `?ov=`      — the orchestration view (medulla | agent | overview | tasks | network)
 *   - `?sub=`     — the network sub-view (connections | discover | usage)
 *   - `?session=` — the open peer session (read by {@link AgentChatPanel})
 *
 * The live list of active peer sessions (the old sidebar "Active sub-agents"
 * rail) moves into the **Chat** view as a left column, since clicking a session
 * opens it in that chat anyway.
 */
import { useCallback, useMemo } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import { useMedullaAccess } from '../../lib/orchestration/useMedullaAccess';
import { useContactSessions } from '../../lib/orchestration/useOrchestrationSessions';
import ChipTabs from '../layout/ChipTabs';
import PageSectionHeader from '../layout/PageSectionHeader';
import PanelPage from '../layout/PanelPage';
import ActiveSubagentsRail from './ActiveSubagentsRail';
import AgentChatPanel from './AgentChatPanel';
import ConnectionsPanel from './ConnectionsPanel';
import MedullaDemoChat from './demo/MedullaDemoChat';
import MedullaDemoGraph from './demo/MedullaDemoGraph';
import MedullaDemoNetwork from './demo/MedullaDemoNetwork';
import DiscoverPanel from './DiscoverPanel';
import MedullaOverviewPanel from './MedullaOverviewPanel';
import OrchestratorTaskBoard from './OrchestratorTaskBoard';
import OverviewPanel from './OverviewPanel';
import UsagePanel from './UsagePanel';

type OrchestrationTab = 'medulla' | 'overview' | 'agent' | 'tasks' | 'network';
type NetworkSub = 'connections' | 'discover' | 'usage';

const ORCH_TABS: readonly OrchestrationTab[] = ['medulla', 'agent', 'overview', 'tasks', 'network'];
const NETWORK_SUBS: readonly NetworkSub[] = ['connections', 'discover', 'usage'];

export default function OrchestrationView() {
  const { t } = useT();
  const location = useLocation();
  const navigate = useNavigate();
  const contactSessions = useContactSessions();
  // Without Medulla access, Medulla-specific live surfaces are replaced by a
  // scale showcase. The global task board remains available to every user.
  const hasMedullaAccess = useMedullaAccess();

  const params = useMemo(() => new URLSearchParams(location.search), [location.search]);
  const rawOv = params.get('ov');
  const rawSub = params.get('sub');

  // Default landing is the Medulla overview (the orchestration overview page).
  const activeTab: OrchestrationTab = (ORCH_TABS as readonly string[]).includes(rawOv ?? '')
    ? (rawOv as OrchestrationTab)
    : 'medulla';

  const networkSub: NetworkSub = NETWORK_SUBS.includes(rawSub as NetworkSub)
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
        // Preserve the hosting Brain sub-tab; only the orchestration view moves.
        p.set('tab', 'orchestration');
        p.set('ov', tab);
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
        p.set('tab', 'orchestration');
        p.set('ov', 'network');
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
        p.set('tab', 'orchestration');
        p.set('ov', 'agent');
        if (sessionId) p.set('session', sessionId);
        else p.delete('session');
      });
    },
    [updateParams]
  );

  console.debug(
    '[orchestration] view mount ov=%s sub=%s session=%s medullaAccess=%s',
    activeTab,
    networkSub,
    openSessionId,
    hasMedullaAccess
  );

  const chipItems = useMemo(
    () => [
      { id: 'medulla' as const, label: t('orchPage.medulla.nav') },
      { id: 'agent' as const, label: t('orchPage.agent.nav') },
      { id: 'overview' as const, label: t('orchPage.overview.nav') },
      { id: 'tasks' as const, label: t('orchPage.tasks.nav') },
      { id: 'network' as const, label: t('orchPage.group.network') },
    ],
    [t]
  );

  return (
    <div className="flex h-full flex-col">
      {/* Top-level view switcher — the old sidebar rail, folded into an in-content
          chip row so Brain keeps a single sidebar. */}
      <div className="shrink-0 border-b border-line-subtle">
        <ChipTabs<OrchestrationTab>
          as="tab"
          ariaLabel={t('nav.orchestration')}
          testIdPrefix="orch-view"
          items={chipItems}
          value={activeTab}
          onChange={setActiveTab}
        />
      </div>

      <div className="min-h-0 flex-1">
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
          // Full-bleed so it reads exactly like the normal chat page — or a
          // read-only demo conversation (composer disabled) without Medulla
          // access. The active peer-session list sits in a left column (moved
          // from the former sidebar rail) when there are live sessions.
          hasMedullaAccess ? (
            <div className="flex h-full">
              {contactSessions.byContact.size > 0 ? (
                <aside className="hidden w-60 shrink-0 overflow-y-auto border-r border-line-subtle px-1.5 py-2 lg:block">
                  <ActiveSubagentsRail
                    byContact={contactSessions.byContact}
                    openSessionId={openSessionId}
                    isAgentTab={true}
                    onOpenSession={setOpenSessionId}
                  />
                </aside>
              ) : null}
              <div className="min-w-0 flex-1">
                <AgentChatPanel openSessionId={openSessionId} onOpenSession={setOpenSessionId} />
              </div>
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
    </div>
  );
}
