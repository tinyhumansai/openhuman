/**
 * Reflection card list for the Intelligence tab (#623).
 *
 * Self-contained component that polls `subconscious_reflections_list`,
 * renders a card per reflection (Observe + Notify) with a disposition
 * badge, action button (only meaningful for Notify reflections with a
 * `proposed_action`), and dismiss button. Optimistic dismiss hides the
 * card immediately on tap so the UI feels responsive.
 *
 * Acting on a reflection drives `actOnReflection` which routes to the
 * user's *active orchestrator thread* (passed in via props, NOT the
 * subconscious thread) so the conversation moves into the user's
 * normal chat surface.
 */
import { useCallback, useEffect, useState } from 'react';

import {
  actOnReflection,
  dismissReflection,
  listReflections,
  type Reflection,
  type ReflectionDisposition,
  type ReflectionKind,
} from '../../utils/tauriCommands/subconscious';

interface SubconsciousReflectionCardsProps {
  /**
   * The user's active orchestrator thread id. Taps on a proposed
   * action route the conversation here — NOT into the subconscious
   * thread itself.
   */
  activeThreadId: string | null;
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

const DISPOSITION_LABEL: Record<ReflectionDisposition, string> = {
  observe: 'Observed',
  notify: 'In conversation',
};

export default function SubconsciousReflectionCards({
  activeThreadId,
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
    if (!activeThreadId) {
      console.debug('[subconscious-ui] reflection act:skipped no activeThreadId', {
        id: reflection.id,
      });
      setError('No active conversation thread to act in.');
      return;
    }
    console.debug('[subconscious-ui] reflection act:start', {
      id: reflection.id,
      target: activeThreadId,
    });
    try {
      const resp = await actOnReflection(reflection.id, activeThreadId);
      console.debug('[subconscious-ui] reflection act:ok', {
        id: reflection.id,
        request: resp.result.request_id,
      });
      setHiddenIds(prev => new Set(prev).add(reflection.id));
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

  return (
    <div data-testid="reflection-cards" className="space-y-2">
      <h3 className="text-sm font-semibold text-stone-900 mb-3 flex items-center gap-2">
        <span className="w-2 h-2 rounded-full bg-primary-400" />
        Reflections
        <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-primary-50 text-primary-700">
          {visible.length}
        </span>
      </h3>
      {error && (
        <div data-testid="reflection-cards-error" className="text-xs text-coral-600 mb-2">
          {error}
        </div>
      )}
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
                  className={`text-[10px] px-2 py-0.5 rounded-full ${
                    r.disposition === 'notify'
                      ? 'bg-primary-100 text-primary-700'
                      : 'bg-stone-100 text-stone-500'
                  }`}>
                  {DISPOSITION_LABEL[r.disposition] ?? r.disposition}
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
              {r.disposition === 'notify' && r.proposed_action && (
                <button
                  data-testid={`reflection-act-${r.id}`}
                  onClick={() => void handleAct(r)}
                  disabled={!activeThreadId}
                  className="px-3 py-1.5 text-xs bg-primary-500 hover:bg-primary-600 disabled:opacity-40 text-white rounded-lg transition-colors">
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
  );
}
