/**
 * Brain — the centerpiece memory surface.
 *
 * Reached from the raised center button in the bottom bar. On open it plays a
 * short "brain collecting information" Lottie flourish (a minimum display
 * window so it always reads as intentional), then cross-fades into the live
 * knowledge graph once its force layout has settled. Below the graph it
 * surfaces the full memory workspace — controls, tree status, and connected
 * sources — framed as a clean, single-column dashboard.
 *
 * Readiness gate — the loading overlay leaves only when ALL of:
 *   1. the minimum animation window has elapsed (skipped under reduced motion),
 *   2. the graph data has been fetched, and
 *   3. the graph layout has settled (`MemoryGraph` `onReady`, or there's nothing
 *      to lay out) — OR a hard max-timeout fires so a stuck layout can never
 *      trap the user behind the overlay, OR the fetch errored.
 *
 * Everything here is light: one existing RPC for the graph (the status/sources
 * panels own their polling), the existing Pixi/worker graph pipeline, and a
 * runtime-fetched (cached) Lottie asset.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import { MemoryControls } from '../components/intelligence/MemoryControls';
import { MemoryGraph } from '../components/intelligence/MemoryGraph';
import { MemorySourcesRegistry } from '../components/intelligence/MemorySourcesRegistry';
import { MemoryTreeStatusPanel } from '../components/intelligence/MemoryTreeStatusPanel';
import { ToastContainer } from '../components/intelligence/Toast';
import LottieAnimation from '../components/LottieAnimation';
import { useT } from '../lib/i18n/I18nContext';
import type { ToastNotification } from '../types/intelligence';
import {
  type GraphExportResponse,
  type GraphMode,
  memoryTreeGraphExport,
} from '../utils/tauriCommands';

/** Minimum time the loading flourish stays up so it never just flashes. */
const MIN_ANIMATION_MS = 1200;
/** Hard ceiling: reveal the graph regardless after this, even if not settled. */
const MAX_WAIT_MS = 8000;
const BRAIN_LOTTIE_SRC = '/lottie/brain-collecting.json';

/** Honour the OS reduced-motion preference — skip the flourish entirely. */
const prefersReducedMotion = (): boolean =>
  typeof window !== 'undefined' &&
  typeof window.matchMedia === 'function' &&
  window.matchMedia('(prefers-reduced-motion: reduce)').matches;

export default function Brain() {
  const { t } = useT();
  const [graph, setGraph] = useState<GraphExportResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Graph layout has settled (or there is nothing to lay out).
  const [graphReady, setGraphReady] = useState(false);
  // Minimum animation window has elapsed.
  const [minElapsed, setMinElapsed] = useState(false);
  // Hard ceiling reached — reveal regardless.
  const [timedOut, setTimedOut] = useState(false);
  // Graph view mode — driven by the toolbar's Trees / Contacts toggle.
  const [mode, setMode] = useState<GraphMode>('tree');
  // Bumped to force a graph re-pull (Refresh + post-mutation refresh).
  const [refreshKey, setRefreshKey] = useState(0);
  const [toasts, setToasts] = useState<ToastNotification[]>([]);

  const reduceMotion = useRef(prefersReducedMotion());

  const addToast = useCallback((toast: Omit<ToastNotification, 'id'>) => {
    setToasts(prev => [...prev, { ...toast, id: `toast-${Date.now()}-${Math.random()}` }]);
  }, []);
  const removeToast = useCallback((id: string) => {
    setToasts(prev => prev.filter(toast => toast.id !== id));
  }, []);
  const refresh = useCallback(() => setRefreshKey(k => k + 1), []);

  // ── Fetch the graph on mount / mode change / refresh, and refresh when the
  //    tree finishes building.
  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      console.debug('[brain] graph fetch: entry mode=%s', mode);
      // Clear any prior error so a successful retry isn't masked by a stale one.
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
        // Nothing to lay out → ready immediately (MemoryGraph won't fire onReady
        // for an empty graph since it short-circuits to the empty placeholder).
        if (resp.nodes.length === 0) setGraphReady(true);
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
  }, [mode, refreshKey]);

  // ── Minimum-display timer (skipped under reduced motion).
  useEffect(() => {
    if (reduceMotion.current) {
      setMinElapsed(true);
      return;
    }
    const id = window.setTimeout(() => setMinElapsed(true), MIN_ANIMATION_MS);
    return () => window.clearTimeout(id);
  }, []);

  // ── Safety ceiling so the overlay can't trap the user behind a stuck layout.
  useEffect(() => {
    const id = window.setTimeout(() => {
      console.debug('[brain] readiness max-timeout reached');
      setTimedOut(true);
    }, MAX_WAIT_MS);
    return () => window.clearTimeout(id);
  }, []);

  const handleGraphReady = useCallback(() => {
    console.debug('[brain] graph onReady');
    setGraphReady(true);
  }, []);

  const dataLoaded = graph !== null || error !== null;
  const overlayDone = (minElapsed && dataLoaded && graphReady) || error !== null || timedOut;

  // Matches the MemorySourcesRegistry card so the dashboard reads as one set.
  const cardClass =
    'rounded-lg border border-stone-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-900';

  return (
    // Standard page shell — matches Activity/other pages. `relative` anchors the
    // loading overlay and the toast stack.
    <div className="relative min-h-full p-4 pt-6">
      <div className="mx-auto max-w-4xl space-y-5">
        {/* Page header */}
        <header className="min-w-0">
          <h1 className="text-xl font-bold text-stone-900 dark:text-neutral-100">
            {t('nav.brain')}
          </h1>
          <p className="mt-1 text-sm text-stone-500 dark:text-neutral-400">{t('brain.subtitle')}</p>
        </header>

        {/* Graph toolbar — mode toggle + actions. */}
        <MemoryControls
          mode={mode}
          onModeChange={setMode}
          onRefresh={refresh}
          onToast={addToast}
          contentRootAbs={graph?.content_root_abs}
        />

        {/* Hero graph — mounted as soon as data arrives (even while the overlay
            covers it) so its force layout settles and fires onReady underneath. */}
        <div
          className={`transition-opacity duration-700 ${overlayDone ? 'opacity-100' : 'opacity-0'}`}
          aria-hidden={!overlayDone}>
          {/* A loaded graph wins over a transient error so a failed background
              refetch never clobbers an already-rendered graph. */}
          {graph ? (
            <div className="animate-fade-in">
              <MemoryGraph
                nodes={graph.nodes}
                edges={graph.edges}
                mode={mode}
                emptyHint={t('brain.empty')}
                onReady={handleGraphReady}
              />
            </div>
          ) : error ? (
            <div className={`${cardClass} text-sm text-coral-600 dark:text-coral-400`} role="alert">
              {t('brain.error')}
            </div>
          ) : null}
        </div>

        {/* Memory workspace — revealed once the graph is in. The panels own
            their own polling, so deferring their mount keeps the load light. */}
        {overlayDone ? (
          <div className="animate-fade-in space-y-5">
            <div className={cardClass}>
              <MemoryTreeStatusPanel onToast={addToast} />
            </div>
            <MemorySourcesRegistry onToast={addToast} />
          </div>
        ) : null}
      </div>

      {/* Loading overlay — the "brain collecting information" flourish, dressed
          as an ambient ocean-glow scene: a radial gradient wash, a soft pulsing
          halo, and the brain floating above it. Opaque and viewport-filling so
          the whole page (header, toolbar, graph, and the panels below) stays
          hidden until the graph is ready. `fixed` keeps it centered regardless
          of page height; z-40 sits below the bottom nav bar (z-50) so navigation
          stays available. All motion layers are dropped under reduced motion —
          which falls back to a static knowledge-node glyph instead of the
          looping Lottie. */}
      {!overlayDone && (
        <div
          className="fixed inset-0 z-40 flex flex-col items-center justify-center gap-7 overflow-hidden bg-stone-50 animate-fade-in dark:bg-neutral-950"
          data-testid="brain-loading"
          role="status"
          aria-live="polite">
          {/* Ambient ocean-glow wash filling the viewport behind everything. */}
          <div
            aria-hidden
            className="pointer-events-none absolute inset-0 bg-gradient-radial from-primary-500/15 via-transparent to-transparent dark:from-primary-500/20"
          />

          {/* Brain over a soft pulsing halo. The halo breathes (glow-pulse) and
              the brain floats; both are stilled under reduced motion. */}
          <div className="relative flex h-60 w-60 items-center justify-center">
            <div
              aria-hidden
              className={`absolute h-48 w-48 rounded-full bg-primary-400/30 blur-3xl dark:bg-primary-500/25 ${
                reduceMotion.current ? '' : 'animate-glow-pulse'
              }`}
            />
            {reduceMotion.current ? (
              <BrainGlyph />
            ) : (
              <div className="relative animate-float">
                <LottieAnimation src={BRAIN_LOTTIE_SRC} height={220} width={220} />
              </div>
            )}
          </div>

          <p className="relative text-sm font-medium tracking-wide text-stone-500 dark:text-neutral-400">
            {t('brain.loading')}
          </p>
        </div>
      )}

      <ToastContainer notifications={toasts} onRemove={removeToast} />
    </div>
  );
}

/**
 * Static knowledge-node glyph shown in the loading overlay under reduced
 * motion — a central hub linked to satellite nodes, echoing the graph the
 * page is about to reveal, with no animation.
 */
function BrainGlyph() {
  return (
    <svg
      width={120}
      height={120}
      viewBox="0 0 120 120"
      fill="none"
      className="relative text-primary-500 dark:text-primary-400"
      data-testid="brain-glyph"
      aria-hidden>
      <g stroke="currentColor" strokeWidth={2} opacity={0.45}>
        <line x1="60" y1="60" x2="26" y2="34" />
        <line x1="60" y1="60" x2="96" y2="40" />
        <line x1="60" y1="60" x2="34" y2="92" />
        <line x1="60" y1="60" x2="92" y2="88" />
      </g>
      <g fill="currentColor">
        <circle cx="60" cy="60" r="11" />
        <circle cx="26" cy="34" r="6" opacity={0.85} />
        <circle cx="96" cy="40" r="6" opacity={0.85} />
        <circle cx="34" cy="92" r="6" opacity={0.85} />
        <circle cx="92" cy="88" r="6" opacity={0.85} />
      </g>
    </svg>
  );
}
