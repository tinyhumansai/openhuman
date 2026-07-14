/**
 * Brain — the centerpiece memory + subconscious surface.
 *
 * Two sub-tabs:
 *   - **Memory**: knowledge graph, tree status, and connected sources.
 *   - **Subconscious**: background thinking engine controls.
 */
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import GoalsPanel from '../components/intelligence/GoalsPanel';
import IntelligenceSubconsciousTab from '../components/intelligence/IntelligenceSubconsciousTab';
import { MemoryControls } from '../components/intelligence/MemoryControls';
import { MemoryGraph } from '../components/intelligence/MemoryGraph';
import { MemorySourcesRegistry } from '../components/intelligence/MemorySourcesRegistry';
import { MemoryTreeStatusPanel } from '../components/intelligence/MemoryTreeStatusPanel';
import SubconsciousTriggersPanel from '../components/intelligence/SubconsciousTriggersPanel';
import { SyncAuditPanel } from '../components/intelligence/SyncAuditPanel';
import { ToastContainer } from '../components/intelligence/Toast';
import PageSectionHeader from '../components/layout/PageSectionHeader';
import PageWelcome from '../components/layout/PageWelcome';
import PanelPage from '../components/layout/PanelPage';
import { SidebarContent } from '../components/layout/shell/SidebarSlot';
import TwoPaneNav from '../components/layout/TwoPaneNav';
import BetaBanner from '../components/ui/BetaBanner';
import { useSubconscious } from '../hooks/useSubconscious';
import { useT } from '../lib/i18n/I18nContext';
import { useCoreState } from '../providers/CoreStateProvider';
import type { ToastNotification } from '../types/intelligence';
import {
  type GraphExportResponse,
  type GraphMode,
  memoryTreeGraphExport,
} from '../utils/tauriCommands';

type BrainTab = 'welcome' | 'graph' | 'goals' | 'sources' | 'sync' | 'subconscious';

/** Small inline icon helper for the Brain sidebar nav. */
const navIcon = (d: string) => (
  <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d={d} />
  </svg>
);

const BRAIN_TABS: readonly BrainTab[] = [
  'welcome',
  'graph',
  'goals',
  'sources',
  'sync',
  'subconscious',
];

/** Canonical text header (title + one-line description) per functional tab. */
const BRAIN_HEADERS: Record<Exclude<BrainTab, 'welcome'>, { titleKey: string; descKey: string }> = {
  graph: { titleKey: 'brain.tabs.graph', descKey: 'brain.header.graph' },
  goals: { titleKey: 'brain.tabs.goals', descKey: 'brain.header.goals' },
  sources: { titleKey: 'brain.tabs.sources', descKey: 'brain.header.sources' },
  sync: { titleKey: 'brain.tabs.sync', descKey: 'brain.header.sync' },
  subconscious: { titleKey: 'brain.tabs.subconscious', descKey: 'brain.header.subconscious' },
};

export default function Brain() {
  const { t } = useT();
  const location = useLocation();
  const navigate = useNavigate();
  // Tab is reflected in `?tab=` so deep links (and the redirected old settings
  // routes) land on the right sub-page.
  const activeTab = useMemo<BrainTab>(() => {
    const raw = new URLSearchParams(location.search).get('tab');
    return (BRAIN_TABS as readonly string[]).includes(raw ?? '') ? (raw as BrainTab) : 'welcome';
  }, [location.search]);
  const setActiveTab = useCallback(
    (tab: BrainTab) => {
      const params = new URLSearchParams(location.search);
      params.set('tab', tab);
      navigate({ pathname: location.pathname, search: `?${params.toString()}` });
    },
    [location.pathname, location.search, navigate]
  );
  // Back-compat: TinyPlace Orchestration was promoted out of Brain into the
  // top-level `/orchestration` tab. Bounce the old deep link there.
  useEffect(() => {
    if (new URLSearchParams(location.search).get('tab') === 'tinyplace-orchestration') {
      console.debug('[brain] legacy tinyplace-orchestration deep link → /orchestration');
      navigate('/orchestration', { replace: true });
    }
  }, [location.search, navigate]);

  const [graph, setGraph] = useState<GraphExportResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [mode, setMode] = useState<GraphMode>('tree');
  const [refreshKey, setRefreshKey] = useState(0);
  const [toasts, setToasts] = useState<ToastNotification[]>([]);

  // The memory graph is read from the on-disk store, but the read only fired on
  // mount — so after a logout→login cycle the page kept whatever (empty) state
  // it had when the core was signed-out / mid identity-flip and never refetched
  // once auth was restored, showing an empty graph for an account whose data is
  // still on disk (#4149). Key the load on the authenticated identity so a
  // re-auth (null→user, or A→B) re-pulls the persisted graph, mirroring the
  // thread-cache reload CoreStateProvider already does on identity change.
  const { snapshot } = useCoreState();
  const authUserId = snapshot.auth.userId;

  const sub = useSubconscious();

  const addToast = useCallback((toast: Omit<ToastNotification, 'id'>) => {
    setToasts(prev => [...prev, { ...toast, id: `toast-${Date.now()}-${Math.random()}` }]);
  }, []);
  const removeToast = useCallback((id: string) => {
    setToasts(prev => prev.filter(toast => toast.id !== id));
  }, []);
  const refresh = useCallback(() => setRefreshKey(k => k + 1), []);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      console.debug('[brain] graph fetch: entry mode=%s', mode);
      setError(null);
      try {
        const resp = await memoryTreeGraphExport(mode);
        if (cancelled) return;
        console.debug(
          '[brain] graph fetch: exit n=%d edges=%d',
          resp.nodes.length,
          resp.edges.length
        );
        setGraph(resp);
      } catch (err) {
        if (cancelled) return;
        console.error('[brain] graph fetch failed', err);
        setError(err instanceof Error ? err.message : String(err));
      }
    };
    void load();
    const onTreeDone = () => {
      console.debug('[brain] memory-tree-completed → refetch');
      void load();
    };
    window.addEventListener('openhuman:memory-tree-completed', onTreeDone);
    return () => {
      cancelled = true;
      window.removeEventListener('openhuman:memory-tree-completed', onTreeDone);
    };
    // `authUserId` is a dependency so a logout→login (identity becomes
    // available again) re-pulls the persisted graph instead of leaving the
    // signed-out empty state on screen (#4149).
  }, [mode, refreshKey, authUserId]);

  const cardClass = 'rounded-lg border border-line bg-surface p-4';

  return (
    <div className="h-full">
      {/* The Brain navigation lives in the root app sidebar's dynamic region. */}
      <SidebarContent>
        <div className="h-full overflow-hidden">
          <TwoPaneNav
            ariaLabel={t('nav.brain')}
            selected={activeTab}
            onSelect={value => setActiveTab(value as BrainTab)}
            groups={[
              {
                items: [
                  {
                    value: 'welcome',
                    label: t('brain.welcome.nav'),
                    icon: navIcon('M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z'),
                  },
                  {
                    value: 'graph',
                    label: t('brain.tabs.graph'),
                    icon: navIcon(
                      'M8.684 13.342C8.886 12.938 9 12.482 9 12c0-.482-.114-.938-.316-1.342m0 2.684a3 3 0 110-2.684m0 2.684l6.632 3.316m-6.632-6l6.632-3.316m0 0a3 3 0 105.367-2.684 3 3 0 00-5.367 2.684zm0 9.316a3 3 0 105.368 2.684 3 3 0 00-5.368-2.684z'
                    ),
                  },
                  {
                    value: 'goals',
                    label: t('brain.tabs.goals'),
                    icon: navIcon('M5 3v18M5 3l13 4-13 4M5 13l9 3-9 3'),
                  },
                  {
                    value: 'sources',
                    label: t('brain.tabs.sources'),
                    icon: navIcon(
                      'M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4'
                    ),
                  },
                  {
                    value: 'sync',
                    label: t('brain.tabs.sync'),
                    icon: navIcon(
                      'M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15'
                    ),
                  },
                  {
                    value: 'subconscious',
                    label: t('brain.tabs.subconscious'),
                    icon: navIcon(
                      'M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z'
                    ),
                  },
                ],
              },
            ]}
          />
        </div>
      </SidebarContent>
      <div className="mx-auto h-full w-full max-w-5xl">
        {activeTab === 'welcome' ? (
          <PageWelcome
            testId="brain-welcome"
            accent="sage"
            icon="🧠"
            eyebrow={t('brain.welcome.eyebrow')}
            title={t('brain.welcome.title')}
            description={t('brain.welcome.body')}
            ctas={[
              {
                label: t('brain.welcome.ctaGraph'),
                icon: '🕸️',
                onClick: () => setActiveTab('graph'),
                testId: 'brain-welcome-cta-graph',
              },
              {
                label: t('brain.welcome.ctaGoals'),
                icon: '🎯',
                onClick: () => setActiveTab('goals'),
              },
              {
                label: t('brain.welcome.ctaSources'),
                icon: '🔗',
                onClick: () => setActiveTab('sources'),
              },
            ]}
            featuresHeading={t('brain.welcome.featsLabel')}
            features={[
              {
                icon: '🕸️',
                title: t('brain.welcome.feat1Title'),
                description: t('brain.welcome.feat1Body'),
              },
              {
                icon: '🎯',
                title: t('brain.welcome.feat2Title'),
                description: t('brain.welcome.feat2Body'),
              },
              {
                icon: '🔄',
                title: t('brain.welcome.feat3Title'),
                description: t('brain.welcome.feat3Body'),
              },
            ]}
          />
        ) : (
          /* All tabs share the standard scaffold: a single scrolling body,
            all custom controls live inside it. Each tab opens with the canonical
            header card (title + one-line description), aligned to the content. */
          <PanelPage contentClassName="p-4">
            <div className="mx-auto max-w-3xl space-y-5">
              <PageSectionHeader
                title={t(BRAIN_HEADERS[activeTab as Exclude<BrainTab, 'welcome'>].titleKey)}
                description={t(BRAIN_HEADERS[activeTab as Exclude<BrainTab, 'welcome'>].descKey)}
              />
              {activeTab === 'graph' && (
                <div className="space-y-5 animate-fade-up">
                  <MemoryControls
                    mode={mode}
                    onModeChange={setMode}
                    onRefresh={refresh}
                    onToast={addToast}
                    contentRootAbs={graph?.content_root_abs}
                  />

                  {graph ? (
                    <MemoryGraph
                      nodes={graph.nodes}
                      edges={graph.edges}
                      mode={mode}
                      emptyHint={t('brain.empty')}
                    />
                  ) : error ? (
                    <div
                      className={`${cardClass} text-sm text-coral-600 dark:text-coral-400`}
                      role="alert">
                      {t('brain.error')}
                    </div>
                  ) : null}
                </div>
              )}

              {activeTab === 'goals' && <GoalsPanel />}

              {activeTab === 'sources' && (
                <div className="space-y-5 animate-fade-up">
                  <MemorySourcesRegistry onToast={addToast} />
                </div>
              )}

              {activeTab === 'sync' && (
                <div className="space-y-5 animate-fade-up">
                  <div className={cardClass}>
                    <MemoryTreeStatusPanel onToast={addToast} />
                  </div>
                  {/* Sync history relocated from the Memory Inspection panel so
                      the Sync tab is the single sync surface. */}
                  <div className={cardClass} data-testid="brain-sync-history">
                    <h3 className="mb-2 text-sm font-medium text-content-secondary">
                      {t('sync.auditTitle', 'Sync History')}
                    </h3>
                    <SyncAuditPanel />
                  </div>
                </div>
              )}

              {activeTab === 'subconscious' && (
                <div className="space-y-3 animate-fade-up">
                  <BetaBanner />
                  <div className={cardClass}>
                    <IntelligenceSubconsciousTab
                      status={sub.status}
                      instances={sub.instances}
                      mode={sub.mode}
                      intervalMinutes={sub.intervalMinutes}
                      triggerTick={sub.triggerTick}
                      triggering={sub.triggering}
                      isTriggering={sub.isTriggering}
                      settingMode={sub.settingMode}
                      setMode={sub.setMode}
                      setIntervalMinutes={sub.setIntervalMinutes}
                    />
                  </div>
                  <SubconsciousTriggersPanel />
                </div>
              )}
            </div>
          </PanelPage>
        )}
      </div>

      <ToastContainer notifications={toasts} onRemove={removeToast} />
    </div>
  );
}
