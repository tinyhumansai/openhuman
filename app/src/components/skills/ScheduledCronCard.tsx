/**
 * ScheduledCronCard — the polished "scheduled skill" card.
 *
 * Now used by WorkflowRunnerBody.tsx (the per-workflow saved-schedules
 * list at `/skills/run`). It originally also backed a standalone
 * scheduled-jobs dashboard (`SkillsDashboard`, removed when that overview
 * was phased out — see commit c474bc36 for the original inline version),
 * which is why it still supports an optional clickable-card mode.
 *
 * Composition is intentional:
 *
 *   onClick   — if provided, the whole card becomes a clickable button
 *               (was used by the removed dashboard to navigate into the
 *               runner). Absent on the runner page where you're already
 *               on the right skill, so clicking would be a no-op.
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
import { Badge, Switch } from '../ui';
import { formatSchedule } from './scheduledCronFormat';

interface ScheduledCronCardProps {
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
      ? 'border-sage-200 dark:border-sage-500/30 bg-linear-to-br from-sage-50 via-surface to-sage-100 dark:from-sage-500/10 dark:via-surface dark:to-sage-500/5'
      : 'border-line bg-linear-to-br from-surface via-surface-subtle to-surface-muted dark:from-surface dark:via-surface dark:to-surface-strong'
  }`;

  const headingRow = (
    <div className="min-w-0 flex-1">
      <div className="flex items-center gap-2 min-w-0">
        {activeBadge && (
          <Badge
            variant="success"
            data-testid={`${rootId}-active-badge`}
            className="uppercase tracking-wide shrink-0">
            {`★ ${t('settings.skillsRunner.schedule.active')}`}
          </Badge>
        )}
        <span
          data-testid={`${rootId}-title`}
          className={`font-mono text-sm font-semibold truncate ${
            isActive ? 'text-sage-900 dark:text-sage-100' : 'text-content-secondary'
          }`}>
          {heading}
        </span>
        {badgeCount && badgeCount > 1 ? (
          <Badge data-testid={`${rootId}-count-badge`} className="shrink-0">
            ×{badgeCount}
          </Badge>
        ) : null}
      </div>
      <div
        data-testid={`${rootId}-schedule`}
        className="mt-0.5 text-xs text-content-secondary">
        {formatSchedule(job)}
      </div>
      <div className="mt-1 text-[11px] text-content-muted dark:text-content-faint">
        {job.last_run && (
          <span>
            {t('skills.dashboard.lastRun')}: {new Date(job.last_run).toLocaleString()}
            {job.last_status && (
              <Badge
                variant={job.last_status === 'ok' ? 'success' : 'danger'}
                data-testid={`${rootId}-last-status`}
                className="ml-1.5">
                {job.last_status}
              </Badge>
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
    <span className="flex items-center gap-1.5 shrink-0" onClick={e => e.stopPropagation()}>
      <Switch
        id={`${rootId}-toggle-input`}
        data-testid={`${rootId}-toggle`}
        aria-label={job.enabled ? t('skills.dashboard.disable') : t('skills.dashboard.enable')}
        checked={job.enabled}
        disabled={busy}
        onCheckedChange={next => onToggle(next)}
        className="data-[state=checked]:bg-sage-500"
      />
      <span className="text-[10px] text-content-muted min-w-[44px]">
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
        <span className="flex items-center gap-1.5" onClick={e => e.stopPropagation()}>
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
      onKeyDown={e => {
        if (e.key === 'Enter' || e.key === ' ') {
          if (e.key === ' ') e.preventDefault();
          onClick();
        }
      }}
      className="w-full text-left px-4 py-3 flex items-center justify-between gap-3 cursor-pointer rounded-2xl transition-colors hover:bg-surface-subtle/80 dark:hover:bg-surface-muted/70 focus:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500/40">
      {headingRow}
      {rightCluster}
    </div>
  ) : (
    <div
      data-testid={`${rootId}-row`}
      className="w-full px-4 py-3 flex items-center justify-between gap-3">
      {headingRow}
      {rightCluster}
    </div>
  );

  return (
    <div
      key={job.id}
      data-testid={rootId}
      data-active={isActive ? 'true' : 'false'}
      className={containerClass}>
      {upperRow}
      {children}
    </div>
  );
}
