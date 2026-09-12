/**
 * Brain — the centerpiece memory surface.
 *
 * Sub-tabs: Welcome, Graph, Goals, Sources, and Sync.
 */
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import { CodingSessionsCard } from '../components/intelligence/CodingSessionsCard';
import GoalsPanel from '../components/intelligence/GoalsPanel';
import { MemoryControls } from '../components/intelligence/MemoryControls';
import { MemoryGraph } from '../components/intelligence/MemoryGraph';
import { MemorySourcesRegistry } from '../components/intelligence/MemorySourcesRegistry';
import { MemoryTreeStatusPanel } from '../components/intelligence/MemoryTreeStatusPanel';
import { SyncAuditPanel } from '../components/intelligence/SyncAuditPanel';
import { ToastContainer } from '../components/intelligence/Toast';
import PageWelcome from '../components/layout/PageWelcome';
import { SidebarContent } from '../components/layout/shell/SidebarSlot';
import TwoPaneNav from '../components/layout/TwoPaneNav';
import SettingsTabbedPage from '../components/settings/layout/SettingsTabbedPage';
import { Alert, AlertDescription, Card } from '../components/ui';
import { useT } from '../lib/i18n/I18nContext';
import { useCoreState } from '../providers/CoreStateProvider';
import type { ToastNotification } from '../types/intelligence';
import {
  type GraphExportResponse,
  type GraphMode,
  memoryTreeGraphExport,
} from '../utils/tauriCommands';

type BrainTab = 'welcome' | 'graph' | 'goals' | 'sources' | 'sync';

/** Small inline icon helper for the Brain sidebar nav. */
const navIcon = (d: string) => (
  <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d={d} />
  </svg>
);

const BRAIN_TABS: readonly BrainTab[] = ['welcome', 'graph', 'goals', 'sources', 'sync'];

/**
 * Backoff ladder for automatically retrying a failed graph load.
 *
 * Written out rather than computed from an exponent because the two things a
 * reader needs — how long the wait is, and that there are exactly three of
 * them — are then both visible. The last element is the bound: once the ladder
 * is spent the error stays on screen and manual Refresh is the way back.
 */
const RETRY_DELAYS_MS: readonly number[] = [2_000, 4_000, 8_000];

/**
 * Canonical text header (title + one-line description) per functional tab.
 */
const BRAIN_HEADERS: Record<Exclude<BrainTab, 'welcome'>, { titleKey: string; descKey: string }> = {
  graph: { titleKey: 'brain.tabs.graph', descKey: 'brain.header.graph' },
  goals: { titleKey: 'brain.tabs.goals', descKey: 'brain.header.goals' },
  sources: { titleKey: 'brain.tabs.sources', descKey: 'brain.header.sources' },
  sync: { titleKey: 'brain.tabs.sync', descKey: 'brain.header.sync' },
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

  const addToast = useCallback((toast: Omit<ToastNotification, 'id'>) => {
    setToasts(prev => [...prev, { ...toast, id: `toast-${Date.now()}-${Math.random()}` }]);
  }, []);
  const removeToast = useCallback((id: string) => {
    setToasts(prev => prev.filter(toast => toast.id !== id));
  }, []);
  const refresh = useCallback(() => setRefreshKey(k => k + 1), []);

  useEffect(() => {
    let cancelled = false;
    // The pending automatic retry, if one is scheduled. Effect-scoped, so a
    // dependency change or an unmount cancels it in the cleanup below.
    let retryTimer: ReturnType<typeof setTimeout> | undefined;
    // Index into RETRY_DELAYS_MS. Effect-scoped too, which is what makes a
    // manual Refresh (a `refreshKey` change re-runs this effect) start the
    // ladder over rather than inheriting a spent one.
    let attempt = 0;
    // Monotonic request generation. Two `load()` calls can be in flight at once
    // (the initial one and a `memory-tree-completed` event, or an automatic
    // retry overtaken by an event), and they share `graph`, `error` and the
    // retry ladder. Without this, a SUPERSEDED call's late result still writes:
    // an obsolete rejection sets an error and schedules another retry even
    // though a newer call has already succeeded, and an obsolete success can
    // overwrite a newer graph. `cancelled` does not cover this — it only
    // distinguishes this effect run from the next one, never two loads within
    // the same run.
    let generation = 0;
    // Generation of the response currently ON SCREEN, which is NOT the same as
    // the newest request. That difference is the whole point: `generation` says
    // which request is newest, `renderedGeneration` says which one produced the
    // data the user is looking at. They diverge exactly when a newer request
    // FAILED — and that is the case where a superseded success must still be
    // allowed through, because there is no newer data for it to clobber.
    let renderedGeneration = 0;

    const load = async (isAutomaticRetry = false) => {
      const myGeneration = ++generation;
      // Any load supersedes a retry still waiting, so a manual Refresh or a
      // `memory-tree-completed` event cannot race a timer into a double fetch.
      if (retryTimer !== undefined) {
        clearTimeout(retryTimer);
        retryTimer = undefined;
      }
      console.debug(
        '[brain] graph fetch: entry mode=%s attempt=%d retry=%s',
        mode,
        attempt,
        isAutomaticRetry
      );
      // An AUTOMATIC retry must not clear the error while it is in flight.
      // Clearing it here is right for a load the user or the app asked for —
      // mount, Refresh, `memory-tree-completed` — because the previous failure
      // is no longer what is being reported. A timer-driven retry is different:
      // nothing has changed from the user's point of view, so blanking the
      // alert (or the stale-data warning) for the duration of the request makes
      // the failure flicker out and back for no reason they can perceive. The
      // success path clears it on an accepted success, which is when the error
      // has actually stopped being true.
      if (!isAutomaticRetry) setError(null);
      try {
        const resp = await memoryTreeGraphExport(mode);
        // Discard this success only if NEWER DATA already rendered — not merely
        // because a newer request exists. Testing `myGeneration !== generation`
        // here also dropped the older success when the newer request had
        // failed, which leaves the user with an error and no graph: strictly
        // worse than either behaviour this guard was meant to produce.
        if (cancelled || myGeneration < renderedGeneration) {
          console.debug('[brain] graph fetch: dropping success behind newer data');
          return;
        }
        console.debug(
          '[brain] graph fetch: exit n=%d edges=%d',
          resp.nodes.length,
          resp.edges.length
        );
        setGraph(resp);
        renderedGeneration = myGeneration;
        // Clear the error on an ACCEPTED SUCCESS, not only when a load starts.
        // Two `load()` calls can overlap (the initial one and a
        // `memory-tree-completed` event, or two events in quick succession),
        // and they share this state with no request-generation guard. If the
        // newer call fails and the older then succeeds, `error` stays set from
        // the newer one while a perfectly good graph renders — which before
        // this PR was invisible, and with the warning below would be a FALSE
        // "your data is stale" on data that is not.
        setError(null);
        // Success resets the ladder: the NEXT transient failure gets the full
        // set of retries rather than resuming where an old failure left off.
        attempt = 0;
      } catch (err) {
        if (cancelled || myGeneration !== generation) {
          // A newer load has taken over. Dropping this rejection is what stops
          // an obsolete failure from scheduling a retry against a graph that
          // has already refreshed successfully.
          console.debug('[brain] graph fetch: dropping superseded failure');
          return;
        }
        console.error('[brain] graph fetch failed', err);
        setError(err instanceof Error ? err.message : String(err));

        const delay = RETRY_DELAYS_MS[attempt];
        if (delay === undefined) {
          // Bounded on purpose. Past this point the failure is very unlikely to
          // be transient, and retrying forever would hammer the core and hide a
          // real outage behind a spinner. The error stays on screen and the
          // manual Refresh in `MemoryControls` remains the way back.
          console.warn('[brain] graph fetch: retries exhausted after %d attempts', attempt);
          return;
        }
        console.debug('[brain] graph fetch: scheduling retry %d in %dms', attempt + 1, delay);
        attempt += 1;
        retryTimer = setTimeout(() => {
          void load(true);
        }, delay);
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
      if (retryTimer !== undefined) clearTimeout(retryTimer);
      window.removeEventListener('openhuman:memory-tree-completed', onTreeDone);
    };
    // `authUserId` is a dependency so a logout→login (identity becomes
    // available again) re-pulls the persisted graph instead of leaving the
    // signed-out empty state on screen (#4149).
  }, [mode, refreshKey, authUserId]);

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
                ],
              },
            ]}
          />
        </div>
      </SidebarContent>
      {
        // Full width on purpose: the header band has to run edge to edge across
        // the content card, so the width cap cannot live above it. Each tab's
        // body carries its own `mx-auto max-w-3xl`, which is what the old
        // `max-w-5xl` here was really constraining.
        <div className="h-full w-full">
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
            all custom controls live inside it. The title/description go through
            PanelPage so every page opens with the same flush header band, rather
            than a bordered card floating in the content column. */
            <div className="h-full p-4">
              <SettingsTabbedPage
                title={t(BRAIN_HEADERS[activeTab as Exclude<BrainTab, 'welcome'>].titleKey)}
                description={t(BRAIN_HEADERS[activeTab as Exclude<BrainTab, 'welcome'>].descKey)}>
                <div className="w-full space-y-5">
                  {activeTab === 'graph' && (
                    <div className="space-y-5 animate-fade-up">
                      <MemoryControls
                        mode={mode}
                        onModeChange={setMode}
                        onRefresh={refresh}
                        onToast={addToast}
                        contentRootAbs={graph?.content_root_abs}
                      />

                      {/*
                        A failed refresh AFTER a good load keeps the graph on
                        screen and warns, rather than replacing it with an
                        error. The graph is expensive to rebuild and stays
                        useful when a refresh blips, so destroying it would
                        turn a transient failure into total data loss on
                        screen. What is not acceptable is the third option —
                        showing stale data with no indication at all, which is
                        what this did before: the error branch below is
                        reachable only while `graph` is null, and the catch in
                        `load()` never clears `graph`, so a later failure was
                        invisible.

                        The two states are deliberately different components:
                        this one means "what you see is old", the one below
                        means "there is nothing to see".
                      */}
                      {/*
                        `error !== null`, not truthiness: `load()`'s catch does
                        `setError(err.message)`, and an Error carrying an empty
                        message yields `''`, which is falsy. Under a truthiness
                        test that failure suppresses BOTH alerts and is silent
                        again — the exact defect this PR exists to remove.
                      */}
                      {error !== null && graph ? (
                        <Alert variant="warning">
                          <AlertDescription>{t('brain.refreshError')}</AlertDescription>
                        </Alert>
                      ) : null}

                      {graph ? (
                        <MemoryGraph
                          nodes={graph.nodes}
                          edges={graph.edges}
                          mode={mode}
                          emptyHint={t('brain.empty')}
                        />
                      ) : error !== null ? (
                        <Alert variant="destructive">
                          <AlertDescription>{t('brain.error')}</AlertDescription>
                        </Alert>
                      ) : null}
                    </div>
                  )}

                  {activeTab === 'goals' && <GoalsPanel />}

                  {activeTab === 'sources' && (
                    <div className="space-y-5 animate-fade-up">
                      <CodingSessionsCard onToast={addToast} />
                      <MemorySourcesRegistry onToast={addToast} />
                    </div>
                  )}

                  {activeTab === 'sync' && (
                    <div className="space-y-5 animate-fade-up">
                      <Card padded divided={false}>
                        <MemoryTreeStatusPanel onToast={addToast} />
                      </Card>
                      {/* Sync history relocated from the Memory Inspection panel so
                      the Sync tab is the single sync surface. */}
                      <Card padded divided={false} data-testid="brain-sync-history">
                        <h3 className="mb-2 text-sm font-medium text-content-secondary">
                          {t('sync.auditTitle', 'Sync History')}
                        </h3>
                        <SyncAuditPanel />
                      </Card>
                    </div>
                  )}
                </div>
              </SettingsTabbedPage>
            </div>
          )}
        </div>
      }

      <ToastContainer notifications={toasts} onRemove={removeToast} />
    </div>
  );
}
