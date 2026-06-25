import React, { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  type ThreadGoal,
  threadGoalApi,
  type ThreadGoalStatus,
} from '../../../services/api/threadGoalApi';

/**
 * Per-thread goal UI — a Codex-style completion contract the agent pursues
 * across turns. Split into two pieces sharing one {@link useThreadGoal}
 * controller so the trigger can live in the composer footer while the editor
 * opens above the composer:
 *
 * - {@link ThreadGoalFooterTrigger} — a compact affordance in the footer under
 *   the composer ("Set goal" when empty; status + objective when set). Click to
 *   open the editor.
 * - {@link ThreadGoalEditorPanel} — the input field + status/budget + actions,
 *   rendered above the composer, shown only while expanded.
 *
 * Distinct from the global long-term goals list (Intelligence tab) and the
 * thread task board (todo strip). Liveness: fetch on thread change + light poll
 * so agent/continuation-driven changes surface without a manual refresh.
 */

const POLL_INTERVAL_MS = 10_000;

/** Shared controller returned by {@link useThreadGoal}. */
export interface ThreadGoalController {
  threadId: string | null;
  goal: ThreadGoal | null;
  /** Whether the editor panel (above the composer) is open. */
  expanded: boolean;
  draft: string;
  busy: boolean;
  setDraft: (value: string) => void;
  /** Open the editor, seeding the draft from the current objective. */
  open: () => void;
  close: () => void;
  /** Toggle the editor open/closed (open seeds the draft). */
  toggle: () => void;
  /** Persist the draft as the objective (no-op on empty), then collapse. */
  save: () => void;
  complete: () => void;
  pause: () => void;
  resume: () => void;
  clear: () => void;
}

/** Tailwind classes per status, using the app's ocean/sage/amber/coral palette. */
function statusClasses(status: ThreadGoalStatus): string {
  switch (status) {
    case 'active':
      return 'bg-primary-50 text-primary-700 dark:bg-primary-900/40 dark:text-primary-200';
    case 'paused':
      return 'bg-stone-100 text-stone-600 dark:bg-neutral-800 dark:text-neutral-300';
    case 'budget_limited':
      return 'bg-amber-50 text-amber-700 dark:bg-amber-900/40 dark:text-amber-200';
    case 'complete':
      return 'bg-sage-50 text-sage-700 dark:bg-sage-900/40 dark:text-sage-200';
    default:
      return 'bg-stone-100 text-stone-600';
  }
}

/**
 * Goal state + actions for `threadId`. Call once in the parent and hand the
 * result to both the footer trigger and the editor panel so they stay in sync.
 */
export function useThreadGoal(
  threadId: string | null,
  api: typeof threadGoalApi = threadGoalApi
): ThreadGoalController {
  const [goal, setGoal] = useState<ThreadGoal | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [draft, setDraft] = useState('');
  const [busy, setBusy] = useState(false);
  // Guard against thread-switch races resolving onto the wrong thread.
  const activeThread = useRef(threadId);

  const refresh = useCallback(async () => {
    if (!threadId) {
      setGoal(null);
      return;
    }
    try {
      const g = await api.get(threadId);
      if (activeThread.current === threadId) setGoal(g);
    } catch {
      /* best-effort; keep last known goal */
    }
  }, [api, threadId]);

  // Reset the editor + cached goal when the thread changes. Done during render
  // (React's sanctioned "reset state on prop change" pattern) rather than in an
  // effect, so it's synchronous and lint-clean.
  if (activeThread.current !== threadId) {
    activeThread.current = threadId;
    setExpanded(false);
    setGoal(null);
  }

  // Fetch on mount/thread-change and poll lightly. `refresh` is async, so its
  // setState lands in a later microtask (not a synchronous effect write).
  useEffect(() => {
    if (!threadId) return;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- async fetch; setState lands post-await
    void refresh();
    const id = window.setInterval(() => void refresh(), POLL_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [threadId, refresh]);

  const runAction = useCallback(
    async (fn: () => Promise<ThreadGoal | null | boolean>, collapse: boolean) => {
      setBusy(true);
      try {
        await fn();
        await refresh();
        if (collapse) setExpanded(false);
      } finally {
        setBusy(false);
      }
    },
    [refresh]
  );

  const open = useCallback(() => {
    setDraft(goal?.objective ?? '');
    setExpanded(true);
  }, [goal]);

  const close = useCallback(() => setExpanded(false), []);

  const toggle = useCallback(() => {
    setExpanded(prev => {
      if (!prev) setDraft(goal?.objective ?? '');
      return !prev;
    });
  }, [goal]);

  const save = useCallback(() => {
    if (!threadId) return;
    const objective = draft.trim();
    if (!objective) return;
    void runAction(() => api.set(threadId, objective), true);
  }, [api, draft, runAction, threadId]);

  const complete = useCallback(() => {
    if (threadId) void runAction(() => api.complete(threadId), true);
  }, [api, runAction, threadId]);
  const pause = useCallback(() => {
    if (threadId) void runAction(() => api.pause(threadId), false);
  }, [api, runAction, threadId]);
  const resume = useCallback(() => {
    if (threadId) void runAction(() => api.resume(threadId), false);
  }, [api, runAction, threadId]);
  const clear = useCallback(() => {
    if (threadId) void runAction(() => api.clear(threadId), true);
  }, [api, runAction, threadId]);

  return {
    threadId,
    goal,
    expanded,
    draft,
    busy,
    setDraft,
    open,
    close,
    toggle,
    save,
    complete,
    pause,
    resume,
    clear,
  };
}

/** Compact trigger for the composer footer: "Set goal" or the current goal. */
export function ThreadGoalFooterTrigger({
  ctl,
}: {
  ctl: ThreadGoalController;
}): React.ReactElement | null {
  const { t } = useT();
  if (!ctl.threadId) return null;

  if (!ctl.goal) {
    return (
      <button
        type="button"
        onClick={ctl.toggle}
        aria-expanded={ctl.expanded}
        className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-xs text-stone-500 hover:bg-stone-100 hover:text-stone-700 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-neutral-200">
        <span aria-hidden>◎</span>
        {t('conversations.threadGoal.setCta')}
      </button>
    );
  }

  return (
    <button
      type="button"
      onClick={ctl.toggle}
      aria-expanded={ctl.expanded}
      title={ctl.goal.objective}
      className="inline-flex min-w-0 items-center gap-1.5 rounded-md px-1.5 py-0.5 text-xs hover:bg-stone-100 dark:hover:bg-neutral-800">
      <span aria-hidden className="shrink-0 text-stone-400">
        ◎
      </span>
      <span
        className={`shrink-0 rounded px-1 py-0.5 text-[10px] font-medium uppercase tracking-wide ${statusClasses(ctl.goal.status)}`}>
        {t(`conversations.threadGoal.status.${ctl.goal.status}`)}
      </span>
      <MarqueeText
        text={ctl.goal.objective}
        className="max-w-[18rem] text-stone-600 dark:text-neutral-300"
      />
    </button>
  );
}

/** Expanded editor above the composer: input + budget + lifecycle actions. */
export function ThreadGoalEditorPanel({
  ctl,
}: {
  ctl: ThreadGoalController;
}): React.ReactElement | null {
  const { t } = useT();
  if (!ctl.threadId || !ctl.expanded) return null;

  const goal = ctl.goal;
  const budgetText =
    goal && typeof goal.tokenBudget === 'number' && goal.tokenBudget > 0
      ? `${goal.tokensUsed.toLocaleString()} / ${goal.tokenBudget.toLocaleString()} ${t('conversations.threadGoal.tokensSuffix')}`
      : null;

  return (
    <div className="flex flex-col gap-1.5 rounded-xl border border-stone-200 bg-stone-50 px-3 py-2 dark:border-neutral-800 dark:bg-neutral-800">
      {/* Controls row — lifecycle (left) + budget and Cancel/Save (right) —
          sits above the input so the input gets the full width below. */}
      <div className="flex items-center gap-1">
        {goal && goal.status === 'active' && (
          <PanelButton
            label={t('conversations.threadGoal.pause')}
            disabled={ctl.busy}
            onClick={ctl.pause}
          />
        )}
        {goal && goal.status === 'paused' && (
          <PanelButton
            label={t('conversations.threadGoal.resume')}
            disabled={ctl.busy}
            onClick={ctl.resume}
          />
        )}
        {goal && goal.status !== 'complete' && (
          <PanelButton
            label={t('conversations.threadGoal.complete')}
            disabled={ctl.busy}
            onClick={ctl.complete}
          />
        )}
        {goal && (
          <PanelButton
            label={t('conversations.threadGoal.clear')}
            disabled={ctl.busy}
            onClick={ctl.clear}
          />
        )}
        <div className="ml-auto flex items-center gap-1">
          {budgetText && (
            <span className="shrink-0 text-[11px] tabular-nums text-stone-400 dark:text-neutral-500">
              {budgetText}
            </span>
          )}
          <button
            type="button"
            onClick={ctl.close}
            className="shrink-0 rounded px-2 py-0.5 text-xs text-stone-500 hover:bg-stone-100 dark:text-neutral-400 dark:hover:bg-neutral-800">
            {t('conversations.threadGoal.cancel')}
          </button>
          <button
            type="button"
            onClick={ctl.save}
            disabled={!ctl.draft.trim() || ctl.busy}
            className="shrink-0 rounded px-2 py-0.5 text-xs font-medium text-primary-600 hover:bg-primary-50 disabled:opacity-40 dark:text-primary-300 dark:hover:bg-primary-900/40">
            {t('conversations.threadGoal.save')}
          </button>
        </div>
      </div>

      {/* Full-width objective input below the controls. */}
      <input
        autoFocus
        value={ctl.draft}
        onChange={e => ctl.setDraft(e.target.value)}
        onKeyDown={e => {
          if (e.key === 'Enter') ctl.save();
          if (e.key === 'Escape') ctl.close();
        }}
        placeholder={t('conversations.threadGoal.placeholder')}
        aria-label={t('conversations.threadGoal.placeholder')}
        className="w-full border-0 bg-transparent text-sm text-stone-800 outline-none focus:outline-none focus:ring-0 placeholder:text-stone-400 dark:text-neutral-100"
      />
    </div>
  );
}

/**
 * Single-line label that gently marquees (ping-pong scroll) only when the text
 * overflows its max width; otherwise it truncates. The scroll distance is
 * measured and fed to the `goal-marquee` keyframe via a CSS variable.
 */
function MarqueeText({
  text,
  className = '',
}: {
  text: string;
  className?: string;
}): React.ReactElement {
  const outerRef = useRef<HTMLSpanElement>(null);
  const innerRef = useRef<HTMLSpanElement>(null);
  const [shift, setShift] = useState(0);

  useEffect(() => {
    const outer = outerRef.current;
    const inner = innerRef.current;
    if (!outer || !inner) return;
    const overflow = inner.scrollWidth - outer.clientWidth;
    setShift(overflow > 4 ? overflow + 8 : 0);
  }, [text]);

  return (
    <span ref={outerRef} className={`relative block min-w-0 overflow-hidden ${className}`}>
      <span
        ref={innerRef}
        className={`block whitespace-nowrap ${shift > 0 ? 'animate-goal-marquee' : 'truncate'}`}
        style={
          shift > 0 ? ({ '--goal-marquee-shift': `-${shift}px` } as React.CSSProperties) : undefined
        }>
        {text}
      </span>
    </span>
  );
}

function PanelButton({
  label,
  onClick,
  disabled,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
}): React.ReactElement {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      onClick={onClick}
      disabled={disabled}
      className="rounded px-1.5 py-0.5 text-[11px] text-stone-500 hover:bg-stone-100 disabled:opacity-40 dark:text-neutral-400 dark:hover:bg-neutral-800">
      {label}
    </button>
  );
}
