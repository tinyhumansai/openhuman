/**
 * Reflection card list for the Intelligence tab (#623).
 *
 * Self-contained component that polls `subconscious_reflections_list`,
 * renders a card per reflection with kind chip, action button (only when
 * `proposed_action` is non-null), and dismiss button. Optimistic dismiss
 * hides the card immediately on tap so the UI feels responsive.
 *
 * Acting on a reflection drives `actOnReflection`, which **spawns a fresh
 * conversation thread** seeded with body + proposed_action and returns
 * the new thread id. The component navigates the user (via the
 * `onNavigateToThread` callback) into the new conversation. Reflections
 * never write into existing threads — every act gets its own thread so
 * the user's main chat surface stays uncluttered.
 */
import { useCallback, useEffect, useState } from 'react';

import {
  actOnReflection,
  dismissReflection,
  listReflections,
  type Reflection,
  type ReflectionKind,
} from '../../utils/tauriCommands/subconscious';

interface SubconsciousReflectionCardsProps {
  /**
   * Called after a successful "Act" with the freshly-spawned thread id.
   * Caller is responsible for routing the user into the new conversation
   * (e.g. setting active thread + navigating to the chat surface).
   */
  onNavigateToThread?: (threadId: string) => void;
  /**
   * Polling interval (ms). 0 disables polling — the component will
   * fetch once on mount.
   */
  pollIntervalMs?: number;
  /**
   * Test-only seed used by Vitest to bypass the Tauri RPC layer. When
   * provided, the component renders these reflections without polling.
   */
  initialReflections?: Reflection[];
}

const KIND_LABEL: Record<ReflectionKind, string> = {
  hotness_spike: 'Hotness spike',
  cross_source_pattern: 'Cross-source pattern',
  daily_digest: 'Daily digest',
  due_item: 'Due item',
  risk: 'Risk',
  opportunity: 'Opportunity',
};

/**
 * Render a `created_at` (epoch seconds, as Rust serializes `f64` from
 * `subconscious_reflections.created_at`) into a short relative-time
 * label like "Just now", "5m ago", "3h ago", "2d ago". Anything older
 * than ~7 days falls back to a fixed `MMM D` so cards aren't ambiguous
 * when the user scrolls into older reflections.
 */
function formatRelativeTime(epochSeconds: number): string {
  const nowMs = Date.now();
  const tsMs = epochSeconds * 1000;
  const diffSec = Math.max(0, Math.round((nowMs - tsMs) / 1000));
  if (diffSec < 45) return 'Just now';
  if (diffSec < 3600) return `${Math.round(diffSec / 60)}m ago`;
  if (diffSec < 86_400) return `${Math.round(diffSec / 3600)}h ago`;
  if (diffSec < 604_800) return `${Math.round(diffSec / 86_400)}d ago`;
  return new Date(tsMs).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}

/** Full ISO-ish datetime for the title-attribute tooltip. */
function formatAbsoluteTime(epochSeconds: number): string {
  return new Date(epochSeconds * 1000).toLocaleString();
}

export default function SubconsciousReflectionCards({
  onNavigateToThread,
  pollIntervalMs = 0,
  initialReflections,
}: SubconsciousReflectionCardsProps) {
  const [reflections, setReflections] = useState<Reflection[]>(initialReflections ?? []);
  const [hiddenIds, setHiddenIds] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(initialReflections === undefined);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (initialReflections !== undefined) return; // test mode
    try {
      const resp = await listReflections(50);
      const data = resp.result ?? [];
      console.debug('[subconscious-ui] reflections list:ok', { count: data.length });
      setReflections(data);
      setError(null);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.debug('[subconscious-ui] reflections list:error', { error: msg });
      setError(msg);
    } finally {
      setLoading(false);
    }
  }, [initialReflections]);

  useEffect(() => {
    // Fire the initial fetch through a microtask so `setState` calls
    // inside `refresh` don't run during effect-commit (which trips the
    // `react-hooks/set-state-in-effect` lint).
    let cancelled = false;
    const tick = () => {
      if (cancelled) return;
      void refresh();
    };
    Promise.resolve().then(tick);
    if (pollIntervalMs > 0 && initialReflections === undefined) {
      const handle = setInterval(tick, pollIntervalMs);
      return () => {
        cancelled = true;
        clearInterval(handle);
      };
    }
    return () => {
      cancelled = true;
    };
  }, [refresh, pollIntervalMs, initialReflections]);

  const handleDismiss = async (id: string) => {
    console.debug('[subconscious-ui] reflection dismiss:start', { id });
    setHiddenIds(prev => new Set(prev).add(id)); // optimistic
    try {
      await dismissReflection(id);
      console.debug('[subconscious-ui] reflection dismiss:ok', { id });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.debug('[subconscious-ui] reflection dismiss:error', { id, error: msg });
      // Rollback optimistic hide on failure.
      setHiddenIds(prev => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }
  };

  const handleAct = async (reflection: Reflection) => {
    console.debug('[subconscious-ui] reflection act:start', { id: reflection.id });
    try {
      const resp = await actOnReflection(reflection.id);
      console.debug('[subconscious-ui] reflection act:ok', {
        id: reflection.id,
        thread: resp.result.thread_id,
      });
      setHiddenIds(prev => new Set(prev).add(reflection.id));
      onNavigateToThread?.(resp.result.thread_id);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      console.debug('[subconscious-ui] reflection act:error', { id: reflection.id, error: msg });
      setError(msg);
    }
  };

  const visible = reflections.filter(
    r => !hiddenIds.has(r.id) && r.dismissed_at === null && r.acted_on_at === null
  );

  if (loading) {
    return (
      <div data-testid="reflection-cards-loading" className="text-xs text-stone-400 py-2">
        Loading reflections…
      </div>
    );
  }

  if (visible.length === 0 && !error) {
    return (
      <div data-testid="reflection-cards-empty" className="text-xs text-stone-400 py-3">
        No proactive observations yet — they appear after each subconscious tick.
      </div>
    );
  }

  // Nested-scroll layout: header is pinned at the top of the cards section,
  // the card list below scrolls independently inside `flex-1 overflow-y-auto`.
  // `min-h-0` is the Tailwind escape hatch for the flex-overflow gotcha —
  // without it, `flex-1` children with overflow won't actually shrink to
  // the parent's height and the inner scrollbar never engages.
  return (
    <div data-testid="reflection-cards" className="flex flex-col h-full min-h-0 overflow-hidden">
      <div className="shrink-0 pb-3">
        <h3 className="text-sm font-semibold text-stone-900 flex items-center gap-2">
          <span className="w-2 h-2 rounded-full bg-primary-400" />
          Reflections
          <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-primary-50 text-primary-700">
            {visible.length}
          </span>
        </h3>
        {error && (
          <div data-testid="reflection-cards-error" className="text-xs text-coral-600 mt-2">
            {error}
          </div>
        )}
      </div>
      {/*
        Card list. Two height knobs working together:
          * `flex-1 min-h-0` — when an ancestor has a constrained height
            (e.g. a panel with `h-full`), the inner scroll area fills the
            remaining space and `min-h-0` is the flex-overflow escape
            hatch that lets it actually shrink + scroll instead of
            blowing the parent's bounds.
          * `max-h-[70vh]` — when the cards live inside a flow-sized
            container (the current Intelligence tab uses `space-y-6` with
            no `h-full`, so the panel just grows with content), this
            caps the list at roughly the viewport's upper half. On a
            typical laptop the cap is ~720px, which fits ~8 cards
            comfortably; on a 720p display it shrinks to ~500px.
            Either way the inner list scrolls independently of the rest
            of the Subconscious tab once the cap is hit.
      */}
      <div className="flex-1 min-h-0 max-h-[70vh] overflow-y-auto space-y-2 pr-1">
        {visible.map(r => (
          <div
            key={r.id}
            data-testid={`reflection-card-${r.id}`}
            className="bg-white border border-stone-200 rounded-xl p-4">
            <div className="flex items-start justify-between gap-3">
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2 mb-1">
                  <span className="text-[10px] px-2 py-0.5 rounded-full bg-stone-100 text-stone-600">
                    {KIND_LABEL[r.kind] ?? r.kind}
                  </span>
                  <span
                    data-testid={`reflection-timestamp-${r.id}`}
                    className="text-[10px] text-stone-400"
                    title={formatAbsoluteTime(r.created_at)}>
                    {formatRelativeTime(r.created_at)}
                  </span>
                </div>
                <p className="text-sm text-stone-900 whitespace-pre-line break-words">{r.body}</p>
                {r.proposed_action && (
                  <p className="text-xs text-stone-500 mt-2">
                    <em>Proposed action:</em> {r.proposed_action}
                  </p>
                )}
              </div>
              <div className="flex flex-col gap-2 flex-shrink-0">
                {r.proposed_action && (
                  <button
                    data-testid={`reflection-act-${r.id}`}
                    onClick={() => void handleAct(r)}
                    className="px-3 py-1.5 text-xs bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors">
                    Act
                  </button>
                )}
                <button
                  data-testid={`reflection-dismiss-${r.id}`}
                  onClick={() => void handleDismiss(r.id)}
                  className="px-3 py-1.5 text-xs bg-stone-100 hover:bg-stone-200 text-stone-600 rounded-lg transition-colors">
                  Dismiss
                </button>
              </div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
