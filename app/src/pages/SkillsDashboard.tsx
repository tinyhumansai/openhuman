/**
 * /skills — landing dashboard.
 *
 * Lists the user's currently-scheduled skills as DevWorkflowPanel-style
 * "active config" cards (one per cron job whose name starts with the
 * SkillsRunnerBody prefix `skill-run-`). Each card shows the skill_id,
 * a human-readable schedule, last/next run, and an enable/disable
 * toggle that mirrors DevWorkflowPanel:439's update-then-reload pattern
 * verbatim. Click anywhere else on the card → /skills/run?skill=<id>
 * so the user lands in the runner with the right skill pre-picked.
 *
 * The dashboard *only* surfaces cron-scheduled skills. The catalog of
 * available skills, integrations, etc. lives on /skills/run; the
 * dashboard is deliberately a "what's running on a schedule" view so
 * users can see at a glance what their agent is autonomously doing.
 */
import createDebug from 'debug';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import ScheduledCronCard from '../components/skills/ScheduledCronCard';
import { useT } from '../lib/i18n/I18nContext';
import {
  type CoreCronJob,
  openhumanCronList,
  openhumanCronUpdate,
} from '../utils/tauriCommands/cron';

const log = createDebug('app:pages:SkillsDashboard');

/** Same prefix SkillsRunnerBody.tsx uses to namespace its cron jobs. */
const CRON_NAME_PREFIX = 'skill-run-';

/**
 * Legacy: DevWorkflowPanel saved its cron with the literal
 * `dev-workflow-<repo>` name (e.g. `dev-workflow-tinyhumansai-openhuman`)
 * before SkillsRunnerBody introduced the unified `skill-run-` prefix.
 * Recognise both so the dashboard surfaces existing dev-workflow
 * schedules without forcing the user to delete + re-save them through
 * the new runner UI.
 */
const LEGACY_DEV_WORKFLOW_PREFIX = 'dev-workflow-';
const LEGACY_DEV_WORKFLOW_SKILL_ID = 'dev-workflow';

/**
 * Recognise a cron job name as belonging to a skill schedule and return
 * its skill_id. Returns `null` for cron jobs that don't belong on the
 * Runners dashboard (e.g. memory-tree maintenance, channels polling).
 *
 * Two name shapes are recognised today:
 *   - `skill-run-<skill_id>[-input1=v1_input2=v2…]` (current convention,
 *     written by SkillsRunnerBody for every bundled skill incl. the
 *     "new" dev-workflow path)
 *   - `dev-workflow-<repo>` (legacy DevWorkflowPanel naming) → mapped
 *     back to `skill_id = "dev-workflow"`
 */
function recognizeSkillCron(jobName: string): { skillId: string } | null {
  if (jobName.startsWith(CRON_NAME_PREFIX)) {
    const tail = jobName.slice(CRON_NAME_PREFIX.length);
    // Split on the first `-input=` marker (input pairs always contain `=`).
    const eqIdx = tail.indexOf('=');
    if (eqIdx === -1) return { skillId: tail };
    // Walk back from `=` to the last `-` before it — that's the input-pair separator.
    const dashBeforeEq = tail.lastIndexOf('-', eqIdx);
    if (dashBeforeEq === -1) return { skillId: tail };
    return { skillId: tail.slice(0, dashBeforeEq) };
  }
  if (jobName.startsWith(LEGACY_DEV_WORKFLOW_PREFIX)) {
    return { skillId: LEGACY_DEV_WORKFLOW_SKILL_ID };
  }
  return null;
}

/** Group jobs by skill_id and present a single card per skill (newest first). */
interface SkillGroup {
  skillId: string;
  jobs: CoreCronJob[];
  /** The representative job — the most recently active one. */
  primary: CoreCronJob;
}

function groupBySkill(jobs: CoreCronJob[]): SkillGroup[] {
  const byId = new Map<string, CoreCronJob[]>();
  for (const job of jobs) {
    const recognised = recognizeSkillCron(job.name ?? '');
    if (!recognised) continue;
    const bucket = byId.get(recognised.skillId);
    if (bucket) {
      bucket.push(job);
    } else {
      byId.set(recognised.skillId, [job]);
    }
  }
  const groups: SkillGroup[] = [];
  for (const [skillId, list] of byId.entries()) {
    // Pick "primary": enabled-with-most-recent-last_run beats enabled
    // beats disabled, fall back to created_at desc for stability.
    const sorted = [...list].sort((a, b) => {
      if (a.enabled !== b.enabled) return a.enabled ? -1 : 1;
      const aTs = a.last_run ? new Date(a.last_run).getTime() : 0;
      const bTs = b.last_run ? new Date(b.last_run).getTime() : 0;
      if (aTs !== bTs) return bTs - aTs;
      return new Date(b.created_at).getTime() - new Date(a.created_at).getTime();
    });
    groups.push({ skillId, jobs: sorted, primary: sorted[0] });
  }
  // Order skills by primary's enabled-then-last_run; matches the
  // DevWorkflowPanel sort intent (active surface first).
  groups.sort((a, b) => {
    if (a.primary.enabled !== b.primary.enabled) return a.primary.enabled ? -1 : 1;
    const aTs = a.primary.last_run ? new Date(a.primary.last_run).getTime() : 0;
    const bTs = b.primary.last_run ? new Date(b.primary.last_run).getTime() : 0;
    return bTs - aTs;
  });
  return groups;
}

export default function SkillsDashboard() {
  const { t } = useT();
  const navigate = useNavigate();

  const [jobs, setJobs] = useState<CoreCronJob[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // Per-job "busy" key so we can disable the toggle while update is in
  // flight — mirrors CronJobsPanel's `coreBusyKey` pattern.
  const [busyJobId, setBusyJobId] = useState<string | null>(null);

  const loadJobs = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const resp = await openhumanCronList();
      const all = (resp.result ?? []) as CoreCronJob[];
      // Accept both the current `skill-run-` prefix and the legacy
      // `dev-workflow-` naming DevWorkflowPanel uses, via the shared
      // recogniser at the top of the file.
      const filtered = all.filter(j => recognizeSkillCron(j.name ?? '') !== null);
      log('loaded %d skill cron jobs (of %d total)', filtered.length, all.length);
      setJobs(filtered);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      log('loadJobs error: %s', msg);
      setError(msg);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadJobs();
  }, [loadJobs]);

  // Mirror DevWorkflowPanel:439 verbatim — flip enabled, refresh the
  // list. We keep this generic on the job rather than the skill so
  // it works for any cron-backed skill.
  const handleToggle = useCallback(
    async (job: CoreCronJob) => {
      setBusyJobId(job.id);
      try {
        await openhumanCronUpdate(job.id, { enabled: !job.enabled });
        await loadJobs();
      } catch (err: unknown) {
        log('toggle error: %s', err instanceof Error ? err.message : String(err));
      } finally {
        setBusyJobId(null);
      }
    },
    [loadJobs]
  );

  const groups = useMemo(() => groupBySkill(jobs), [jobs]);

  const goCreate = () => navigate('/skills/new');
  const goRun = () => navigate('/skills/run');
  const goRunSkill = (skillId: string) =>
    navigate(`/skills/run?skill=${encodeURIComponent(skillId)}`);

  return (
    <div className="min-h-full flex flex-col">
      <div className="flex-1 flex items-start justify-center p-4 pt-6">
        <div className="w-full max-w-3xl space-y-4">
          {/* Header + CTAs */}
          <div className="flex items-center justify-between gap-2">
            <h1 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
              {t('skills.dashboard.title')}
            </h1>
            <div className="flex items-center gap-2">
              <button
                type="button"
                data-testid="skills-dashboard-create"
                onClick={goCreate}
                className="rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-xs font-medium text-stone-700 dark:text-neutral-200 shadow-soft transition-colors hover:bg-stone-50 dark:hover:bg-neutral-800">
                + {t('skills.dashboard.create')}
              </button>
              <button
                type="button"
                data-testid="skills-dashboard-run"
                onClick={goRun}
                className="rounded-lg bg-primary-500 px-3 py-2 text-xs font-semibold text-white shadow-soft transition-colors hover:bg-primary-600">
                ▷ {t('skills.dashboard.run')}
              </button>
            </div>
          </div>

          {/* Section heading — kept above whatever state the list is in */}
          <h2 className="text-xs font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400 px-1">
            {t('skills.dashboard.scheduledHeading')}
          </h2>

          {loading && (
            <div
              data-testid="skills-dashboard-loading"
              className="rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-6 shadow-soft text-sm text-stone-500 dark:text-neutral-400">
              {t('common.loading')}
            </div>
          )}

          {!loading && error && (
            <div
              data-testid="skills-dashboard-error"
              className="rounded-2xl border border-coral-200 bg-coral-50 dark:bg-coral-500/10 dark:border-coral-500/30 p-4 text-sm">
              <p className="text-coral-800 dark:text-coral-200">
                {t('skills.dashboard.loadError')}: {error}
              </p>
              <button
                type="button"
                onClick={() => void loadJobs()}
                className="mt-2 rounded border border-coral-300 dark:border-coral-500/40 bg-white dark:bg-neutral-900 px-3 py-1.5 text-xs font-medium text-coral-700 dark:text-coral-300 hover:bg-coral-100 dark:hover:bg-coral-500/15">
                {t('common.retry')}
              </button>
            </div>
          )}

          {!loading && !error && groups.length === 0 && (
            <div
              data-testid="skills-dashboard-empty"
              className="rounded-2xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-8 shadow-soft text-center">
              <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                {t('skills.dashboard.emptyTitle')}
              </h3>
              <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">
                {t('skills.dashboard.emptyBody')}
              </p>
              <button
                type="button"
                data-testid="skills-dashboard-empty-cta"
                onClick={goRun}
                className="mt-4 rounded-lg bg-primary-500 px-4 py-2 text-xs font-semibold text-white shadow-soft transition-colors hover:bg-primary-600">
                ▷ {t('skills.dashboard.run')}
              </button>
            </div>
          )}

          {!loading && !error && groups.length > 0 && (
            <div className="space-y-2">
              {groups.map(group => {
                const job = group.primary;
                const isBusy = busyJobId === job.id;
                // testIdRoot keys the rendered testids:
                //   `skill-card-<id>`           — card root
                //   `skill-card-<id>-open`      — clickable surface
                //   `skill-card-<id>-toggle`    — enable/disable switch
                //   `skill-card-<id>-title`     — title/skill_id text
                //   `skill-card-<id>-schedule`  — cronToHuman line
                // Used by SkillsDashboard.test.tsx and any future e2e specs.
                return (
                  <ScheduledCronCard
                    key={group.skillId}
                    job={job}
                    title={group.skillId}
                    badgeCount={group.jobs.length}
                    onToggle={() => void handleToggle(job)}
                    onClick={() => goRunSkill(group.skillId)}
                    testIdRoot={`skill-card-${group.skillId}`}
                    busy={isBusy}
                  />
                );
              })}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
