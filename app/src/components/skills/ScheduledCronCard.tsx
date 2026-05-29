/**
 * ScheduledCronCard — the polished "scheduled skill" card.
 *
 * Originally lived inline inside SkillsDashboard.tsx (the global
 * dashboard at `/skills` — see commit c474bc36) and inside
 * SkillsRunnerBody.tsx (the per-skill saved-schedules list at
 * `/skills/run`). The two surfaces had diverged: the dashboard rendered
 * a roomy, DevWorkflowPanel-style "active config" card with a friendly
 * cronToHuman schedule and a clickable surface that navigates into the
 * runner; the runner rendered a slim row with the raw cron expression
 * and inline action buttons. The user asked for the runner to adopt
 * the dashboard card pattern so the visual language is consistent.
 *
 * This component is the single shared implementation used by both
 * surfaces. Composition is intentional:
 *
 *   onClick   — if provided, the whole card becomes a clickable button
 *               (used by the dashboard to navigate into the runner).
 *               Absent on the runner page where you're already on the
 *               right skill, so clicking would be a no-op.
 *
 *   onToggle  — enable/disable toggle. Wired identically on both
 *               surfaces; visual lifted from DevWorkflowPanel:502-516.
 *
 *   title     — visible heading. Dashboard passes the extracted skill_id;
 *               runner passes the cron job's `name`.
 *
 *   badgeCount    — small "×N" pill for the dashboard's grouped-multi-job
 *                   case.
 *
 *   activeBadge   — "★ Active" pill for the runner's top-most enabled
 *                   schedule (the one cron will tick first).
 *
 *   actions       — render-slot for extra action buttons next to the
 *                   toggle (runner places Run-Now + Remove here).
 *
 *   children      — render-slot below the row for things like the runner's
 *                   per-job history disclosure.
 *
 * The card stays presentation-only: it does NOT manage any toggle / RPC
 * state itself. Callers pass `onToggle(nextEnabled)` and own the
 * round-trip (call cron_update, re-fetch list, re-render with the new
 * job).
 */
import { useT } from '../../lib/i18n/I18nContext';
import type { CoreCronJob } from '../../utils/tauriCommands/cron';

import { formatSchedule } from './scheduledCronFormat';

export interface ScheduledCronCardProps {
  /** The cron job this card represents (drives schedule + enable state). */
  job: CoreCronJob;
  /** Visible heading. Defaults to `job.name ?? job.id`. */
  title?: string;
  /**
   * Optional `×N` count pill — used by the dashboard when multiple cron
   * jobs collapse into one card (one per skill_id).
   */
  badgeCount?: number;
  /**
   * Optional `★ Active` pill — used by the runner to mark the top-most
   * enabled schedule of a skill (the one the cron tick will fire first).
   */
  activeBadge?: boolean;
  /**
   * Toggle handler — invoked with the desired new enabled state. Returns
   * void; the caller's responsible for cron_update + re-fetch.
   */
  onToggle: (nextEnabled: boolean) => void;
  /**
   * Optional whole-card click handler. When present, the card surface
   * becomes clickable (visually too — focus ring, hover affordance) and
   * navigates the user somewhere — currently `/skills/run?skill=<id>` on
   * the dashboard. Absent on the runner page (you're already there).
   */
  onClick?: () => void;
  /** Stable testid root — defaults to `scheduled-cron-${job.id}`. */
  testIdRoot?: string;
  /**
   * Disable the toggle while a parent-level update is in flight. The card
   * itself doesn't track this — it just reflects what the caller knows.
   */
  busy?: boolean;
  /** Render-slot for extra action buttons next to the toggle. */
  actions?: React.ReactNode;
  /** Render-slot for content below the toggle row (e.g. history disclosure). */
  children?: React.ReactNode;
}

/**
 * Polished card for a scheduled cron job. Used on both the global
 * `/skills` dashboard and the per-skill `/skills/run` runner.
 */
export default function ScheduledCronCard({
  job,
  title,
  badgeCount,
  activeBadge,
  onToggle,
  onClick,
  testIdRoot,
  busy,
  actions,
  children,
}: ScheduledCronCardProps) {
  const { t } = useT();
  const isActive = job.enabled;
  const heading = title ?? job.name ?? job.id;
  const rootId = testIdRoot ?? `scheduled-cron-${job.id}`;

  // Visual states match DevWorkflowPanel's active-config card and the
  // dashboard's grouped-skill card.
  const containerClass = `rounded-2xl border shadow-soft transition-colors ${
    isActive
      ? 'border-sage-200 dark:border-sage-500/30 bg-sage-50 dark:bg-sage-500/10'
      : 'border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900'
  }`;

  const headingRow = (
    <div className="min-w-0 flex-1">
      <div className="flex items-center gap-2 min-w-0">
        {activeBadge && (
          <span
            data-testid={`${rootId}-active-badge`}
            className="px-1.5 py-0.5 rounded text-[10px] font-semibold uppercase tracking-wide bg-sage-200 dark:bg-sage-500/30 text-sage-800 dark:text-sage-200 shrink-0"
          >
            {`★ ${t('settings.skillsRunner.schedule.active')}`}
          </span>
        )}
        <span
          data-testid={`${rootId}-title`}
          className={`font-mono text-sm font-semibold truncate ${
            isActive
              ? 'text-sage-900 dark:text-sage-100'
              : 'text-stone-700 dark:text-neutral-200'
          }`}
        >
          {heading}
        </span>
        {badgeCount && badgeCount > 1 ? (
          <span
            data-testid={`${rootId}-count-badge`}
            className="px-1.5 py-0.5 rounded text-[10px] font-medium bg-stone-200 dark:bg-neutral-700 text-stone-700 dark:text-neutral-300 shrink-0"
          >
            ×{badgeCount}
          </span>
        ) : null}
      </div>
      <div
        data-testid={`${rootId}-schedule`}
        className="mt-0.5 text-xs text-stone-600 dark:text-neutral-400"
      >
        {formatSchedule(job)}
      </div>
      <div className="mt-1 text-[11px] text-stone-500 dark:text-neutral-500">
        {job.last_run && (
          <span>
            {t('skills.dashboard.lastRun')}: {new Date(job.last_run).toLocaleString()}
            {job.last_status && (
              <span
                data-testid={`${rootId}-last-status`}
                className={`ml-1.5 px-1 py-0.5 rounded text-[10px] font-medium ${
                  job.last_status === 'ok'
                    ? 'bg-sage-100 dark:bg-sage-500/20 text-sage-700 dark:text-sage-300'
                    : 'bg-coral-100 dark:bg-coral-500/20 text-coral-700 dark:text-coral-300'
                }`}
              >
                {job.last_status}
              </span>
            )}
          </span>
        )}
        {job.last_run && job.next_run && <span className="mx-1">·</span>}
        {job.next_run && (
          <span>
            {t('skills.dashboard.nextRun')}: {new Date(job.next_run).toLocaleString()}
          </span>
        )}
      </div>
    </div>
  );

  // Toggle markup is shared between clickable + non-clickable variants.
  // We wrap it in a stopPropagation span so toggling never bubbles up to
  // a parent card-click handler.
  const toggleBlock = (
    <span
      className="flex items-center gap-1.5 shrink-0"
      onClick={(e) => e.stopPropagation()}
    >
      <button
        type="button"
        role="switch"
        aria-checked={job.enabled}
        aria-label={
          job.enabled ? t('skills.dashboard.disable') : t('skills.dashboard.enable')
        }
        data-testid={`${rootId}-toggle`}
        disabled={busy}
        onClick={() => onToggle(!job.enabled)}
        className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full transition-colors disabled:opacity-50 ${
          job.enabled ? 'bg-sage-500' : 'bg-neutral-300 dark:bg-neutral-600'
        }`}
      >
        <span
          className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow-sm transition-transform mt-0.5 ${
            job.enabled ? 'translate-x-4' : 'translate-x-0.5'
          }`}
        />
      </button>
      <span className="text-[10px] text-stone-500 dark:text-neutral-400 min-w-[44px]">
        {job.enabled ? t('common.enabled') : t('common.disabled')}
      </span>
    </span>
  );

  // The right-edge cluster: extra `actions` (Run Now / Remove on the
  // runner) followed by the toggle. We wrap in a stopPropagation span so
  // clicking an action button on a clickable card doesn't ALSO navigate.
  const rightCluster = (
    <div className="flex items-center gap-1.5 shrink-0">
      {actions && (
        <span
          className="flex items-center gap-1.5"
          onClick={(e) => e.stopPropagation()}
        >
          {actions}
        </span>
      )}
      {toggleBlock}
    </div>
  );

  // If the caller passed onClick, render the upper row as a clickable
  // container. It must NOT be a real <button>: rightCluster contains the
  // toggle/action <button>s, and nested <button> elements are invalid HTML
  // (breaks a11y + interaction). Use a div with role="button" + keyboard
  // handling instead. Otherwise render a plain div — keeps the runner from
  // carrying a giant clickable surface it doesn't need.
  const upperRow = onClick ? (
    <div
      role="button"
      tabIndex={0}
      data-testid={`${rootId}-open`}
      aria-label={t('skills.dashboard.cardOpenRunner')}
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          if (e.key === ' ') e.preventDefault();
          onClick();
        }
      }}
      className="w-full text-left px-4 py-3 flex items-center justify-between gap-3 cursor-pointer focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-500/40 rounded-2xl"
    >
      {headingRow}
      {rightCluster}
    </div>
  ) : (
    <div
      data-testid={`${rootId}-row`}
      className="w-full px-4 py-3 flex items-center justify-between gap-3"
    >
      {headingRow}
      {rightCluster}
    </div>
  );

  return (
    <div
      key={job.id}
      data-testid={rootId}
      data-active={isActive ? 'true' : 'false'}
      className={containerClass}
    >
      {upperRow}
      {children}
    </div>
  );
}
