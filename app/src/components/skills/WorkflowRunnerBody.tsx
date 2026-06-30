// Reusable Skills Runner body.
//
// Generalises across every bundled skill (`github-issue-crusher`,
// `pr-review-shepherd`, `dev-workflow`, plus anything the user installs
// later) — pick one from the dropdown, fill the dynamically-rendered
// inputs (loaded from `openhuman.workflows_describe`), Run Now to
// fire-and-forget a background autonomous run, or Save as a recurring
// cron schedule. Recent runs are listed below with an inline log
// viewer (click-to-expand, auto-tail for in-flight runs).
//
// Used by both the Settings → Developer Options → Skills Runner panel
// AND the top-level /skills page's "Runners" tab (one source of truth;
// the Settings panel is now a thin wrapper around this body).
import createDebug from 'debug';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useSearchParams } from 'react-router-dom';

import { SCHEDULE_PRESETS } from '../../lib/cron/schedulePresets';
import { useT } from '../../lib/i18n/I18nContext';
import {
  type RunLogSlice,
  type ScannedRun,
  type WorkflowDescription,
  type WorkflowRunStarted,
  workflowsApi,
  type WorkflowSummary,
} from '../../services/api/workflowsApi';
import {
  type CoreCronJob,
  type CoreCronRun,
  openhumanCronAdd,
  openhumanCronList,
  openhumanCronRemove,
  openhumanCronRuns,
  openhumanCronUpdate,
} from '../../utils/tauriCommands/cron';
import Button from '../ui/Button';
import CreateSkillModal from './CreateSkillModal';
import BranchPicker from './inputs/BranchPicker';
import RepoPicker from './inputs/RepoPicker';
import { isGithubGateFailure, parseWorkflowRunError } from './preflightGate';
import ScheduledCronCard from './ScheduledCronCard';
import SmartIssuePicker from './SmartIssuePicker';

// Skills that opt out of the generic schema-driven form for a curated
// composite picker. Today only `dev-workflow` qualifies — its inputs
// (repo, upstream, target_branch, fork_owner) all flow from a single
// GitHub repo selection with fork detection.
//
// TODO(picker-schema): replace this hard-coded set with a schema-level
// signal in `skill.toml` — e.g. `[[inputs]] picker = "github-issue"`.
// See docs/skills-runner-unification.md open question 1.
const SMART_PICKER_SKILL_IDS = new Set(['dev-workflow']);
const SMART_PICKER_INPUT_NAMES = new Set(['repo', 'upstream', 'target_branch', 'fork_owner']);

// Input-name conventions that trigger rich pickers instead of the
// default text/number/checkbox controls. Skill authors who use these
// conventional names get the picker for free; nothing in skill.toml
// needs to change. (We pick a generous overlap that covers both
// github-issue-crusher and dev-workflow's input naming.)
const REPO_INPUT_NAMES = new Set(['repo', 'repository', 'upstream', 'fork', 'fork_owner']);
const BRANCH_INPUT_NAMES = new Set([
  'branch',
  'target_branch',
  'base_branch',
  'pr_base',
  'head_branch',
]);

/**
 * Given the form-value map of the currently-selected skill, return the
 * best `owner/name` value to feed a BranchPicker. The convention is
 * "the value of the first repo-shaped input present", with `repo`
 * preferred over `upstream` over the others.
 */
function resolveLinkedRepo(formValues: Record<string, InputValue>): string {
  const priority = ['repo', 'repository', 'upstream', 'fork'];
  for (const k of priority) {
    const v = formValues[k];
    if (typeof v === 'string' && v.includes('/')) return v;
  }
  return '';
}

const log = createDebug('app:skills:WorkflowRunnerBody');

type InputValue = string | number | boolean;

interface RunState {
  status: 'idle' | 'submitting' | 'started' | 'error';
  message?: string;
  result?: WorkflowRunStarted;
}

/** Name prefix used to identify cron jobs owned by this panel (per-skill). */
const CRON_NAME_PREFIX = 'skill-run-';

/** Build the cron-job name for `(skillId, inputs)` — unique per skill +
 * inputs combo so re-scheduling against the same target updates one job
 * instead of stacking duplicates. We hash inputs into a short slug to
 * keep names readable but distinct. */
function buildCronJobName(skillId: string, inputs: Record<string, unknown>): string {
  const keys = Object.keys(inputs).sort();
  const compact = keys
    .map(k => {
      const v = inputs[k];
      if (v === undefined || v === null || v === '') return '';
      const s = typeof v === 'string' ? v : String(v);
      return `${k}=${s.replace(/[^a-zA-Z0-9._-]+/g, '-').slice(0, 24)}`;
    })
    .filter(Boolean)
    .join('_');
  const suffix = compact.length > 0 ? `-${compact}` : '';
  return `${CRON_NAME_PREFIX}${skillId}${suffix}`.slice(0, 80);
}

/** Compose the agent-job prompt that re-fires the workflow run at cron tick. */
function buildAgentPrompt(workflowId: string, inputs: Record<string, unknown>): string {
  const inputLines = Object.entries(inputs)
    .filter(([, v]) => v !== undefined && v !== null && v !== '')
    .map(([k, v]) => `- ${k}: ${typeof v === 'string' ? v : JSON.stringify(v)}`)
    .join('\n');
  return [
    `Run the ${workflowId} workflow via the run_workflow tool (workflow_id: "${workflowId}") with these inputs:`,
    inputLines || '(no inputs)',
    '',
    'Do NOT do the work yourself — call run_workflow and report back the new run_id.',
  ].join('\n');
}

/**
 * Recover the per-schedule inputs from a saved cron job's agent prompt — the
 * `- key: value` block emitted by {@link buildAgentPrompt}. Each scheduled job
 * carries its own inputs, snapshotted at creation time; this reads them back
 * so the job's card can show exactly what that schedule runs with. Returns an
 * empty list when the job has no inputs (or the prompt isn't recognised).
 */
export function parseScheduledInputs(
  prompt?: string | null
): Array<{ key: string; value: string }> {
  if (!prompt) return [];
  const out: Array<{ key: string; value: string }> = [];
  let inBlock = false;
  for (const line of prompt.split('\n')) {
    if (!inBlock) {
      if (line.includes('with these inputs:')) inBlock = true;
      continue;
    }
    const m = line.match(/^-\s+([^:]+):\s*(.*)$/);
    if (!m) break; // first non-input line ("", "(no inputs)", trailing text) ends the block
    out.push({ key: m[1].trim(), value: m[2].trim() });
  }
  return out;
}

// ── Helpers ────────────────────────────────────────────────────────────

/**
 * Default form value for an input based on its declared type. Strings/
 * integers default to empty (renders as placeholder); booleans to false.
 * `runWorkflow` later trims and drops empty optional fields before sending
 * them over the wire.
 */
function defaultForType(type: string): InputValue {
  if (type === 'boolean') return false;
  if (type === 'integer') return '';
  return '';
}

/**
 * Project the form-state map back into the JSON inputs shape `skill_runtime_run`
 * expects: trim strings, coerce integer-typed fields to numbers, drop
 * empty optional fields entirely (so the backend sees them as "not
 * provided" rather than `""`).
 */
function buildInputsPayload(
  description: WorkflowDescription,
  values: Record<string, InputValue>
): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const inp of description.inputs) {
    const raw = values[inp.name];
    if (raw === undefined || raw === null) {
      if (inp.required) {
        // Will fail validation in the submit handler before we even try to
        // send; included here so the project step is total.
        out[inp.name] = '';
      }
      continue;
    }
    if (inp.type === 'boolean') {
      out[inp.name] = Boolean(raw);
      continue;
    }
    if (typeof raw === 'string' && raw.trim() === '') {
      if (inp.required) out[inp.name] = '';
      continue;
    }
    if (inp.type === 'integer') {
      const n = typeof raw === 'number' ? raw : Number(String(raw).trim());
      if (Number.isFinite(n)) {
        out[inp.name] = n;
      } else if (inp.required) {
        out[inp.name] = raw; // let backend reject with a clear error
      }
      continue;
    }
    out[inp.name] = typeof raw === 'string' ? raw.trim() : raw;
  }
  return out;
}

function inferRuntimeRequirement(skill?: WorkflowSummary): 'node' | 'python' | 'all' | null {
  const resources = skill?.resources ?? [];
  const needsNode = resources.some(resource => /\.(?:cjs|js|mjs)$/i.test(resource));
  const needsPython = resources.some(resource => /\.py$/i.test(resource));
  if (needsNode && needsPython) return 'all';
  if (needsNode) return 'node';
  if (needsPython) return 'python';
  return null;
}

// ── Component ──────────────────────────────────────────────────────────

export interface SkillsRunnerBodyProps {
  /**
   * Optional override for the descriptive header text rendered above
   * the skill picker. Defaults to the Settings-panel description so
   * the original placement is unchanged. (Named `headerText` rather
   * than `description` to avoid shadowing the internal `description`
   * state that holds the resolved `WorkflowDescription` for the picked
   * skill.)
   */
  headerText?: string;
  /**
   * Optional override for the outer container className. The default
   * stacks the sections with `space-y-6`; the Settings panel keeps
   * that, while the top-level /skills tab can extend or replace it.
   */
  className?: string;
}

export const WorkflowRunnerBody = ({ headerText, className }: SkillsRunnerBodyProps) => {
  const { t } = useT();

  // Skill catalog (loaded once on mount)
  const [skills, setSkills] = useState<WorkflowSummary[]>([]);
  const [skillsLoading, setSkillsLoading] = useState(false);
  const [skillsError, setSkillsError] = useState<string | null>(null);
  // Edit-this-workflow modal (only meaningful when locked to a workflow).
  const [editOpen, setEditOpen] = useState(false);

  // Active skill + its full description (inputs declared).
  // Pre-seeded from the URL `?workflow=<id>` query so any surface that
  // deep-links to a specific workflow (e.g. clicking a workflow card, which
  // locks the page to it) can land the user with the picker already pointed
  // at the right workflow.
  const [searchParams, setSearchParams] = useSearchParams();
  const initialSkillId = searchParams.get('workflow') ?? '';
  const [selectedSkillId, setSelectedSkillId] = useState(initialSkillId);
  // Locked mode: opened as a specific workflow's view/run page
  // (`?workflow=<id>&lock=1`) or its Schedule action (`&focus=schedule`). The
  // workflow is fixed — the picker is replaced with a read-only label so this
  // page is that one workflow's run/edit/schedule surface.
  const locked =
    !!searchParams.get('workflow') &&
    (searchParams.get('lock') === '1' || searchParams.get('focus') === 'schedule');
  const selectedWorkflow = skills.find(s => s.id === selectedSkillId);
  const [description, setDescription] = useState<WorkflowDescription | null>(null);
  const [descLoading, setDescLoading] = useState(false);
  const [descError, setDescError] = useState<string | null>(null);

  // Form state per input
  const [formValues, setFormValues] = useState<Record<string, InputValue>>({});

  // Run state
  const [run, setRun] = useState<RunState>({ status: 'idle' });

  // Schedule state
  const [schedule, setSchedule] = useState<string>(SCHEDULE_PRESETS[0].value);
  const [savingSchedule, setSavingSchedule] = useState(false);
  const [scheduleError, setScheduleError] = useState<string | null>(null);
  const [scheduleSaved, setScheduleSaved] = useState(false);
  // Timer that auto-clears the "saved" confirmation; held in a ref so we
  // can cancel it on unmount (and before re-arming) to avoid a setState
  // on an unmounted component.
  const scheduleSavedTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (scheduleSavedTimer.current) clearTimeout(scheduleSavedTimer.current);
    },
    []
  );

  // Scheduled jobs owned by this panel (cron_list filtered by name prefix)
  const [scheduledJobs, setScheduledJobs] = useState<CoreCronJob[]>([]);
  const [scheduledJobsLoading, setScheduledJobsLoading] = useState(false);

  // Sort: enabled-with-most-recent-last_run first (this is the
  // "active" surface — same emphasis DevWorkflowPanel:486-647 gives
  // its single configured job). Then enabled jobs with no recorded
  // last_run, then disabled jobs. Within each bucket fall back to
  // created_at desc for stability.
  const sortedScheduledJobs = useMemo(() => {
    const score = (j: CoreCronJob): number => {
      if (j.enabled && j.last_run) return new Date(j.last_run).getTime();
      if (j.enabled) return 0; // enabled but never ran
      return -1; // disabled
    };
    return [...scheduledJobs].sort((a, b) => {
      const sa = score(a);
      const sb = score(b);
      if (sa === sb) {
        return new Date(b.created_at).getTime() - new Date(a.created_at).getTime();
      }
      return sb - sa;
    });
  }, [scheduledJobs]);

  // The job at the top of the sorted list (if any AND enabled) is the
  // "active" schedule and gets prominent treatment in the row render.
  const activeJobId = useMemo<string | null>(() => {
    const top = sortedScheduledJobs[0];
    return top && top.enabled ? top.id : null;
  }, [sortedScheduledJobs]);

  // Per-job run history (lazy-loaded on row expand). Keyed by job_id so
  // we keep history across re-expansions without re-fetching. Each entry
  // tracks { runs, loading, expandedRunId } for that schedule. The
  // expandedRunId is per-job so multiple history sections can each
  // independently expand a different run's output (unlike the cross-
  // skill recent-runs viewer below which is single-expand).
  const [historyState, setHistoryState] = useState<
    Record<
      string,
      { runs: CoreCronRun[]; loading: boolean; expanded: boolean; expandedRunId: number | null }
    >
  >({});

  // Recent runs (skill-scoped if a skill is picked, cross-skill otherwise)
  const [recentRuns, setRecentRuns] = useState<ScannedRun[]>([]);
  const [recentRunsLoading, setRecentRunsLoading] = useState(false);
  const [recentRunsRefreshNonce, setRecentRunsRefreshNonce] = useState(0);
  // Timers for the post-run refresh burst (cleared on unmount). A just-started
  // run's log header is written a beat after `skill_runtime_run` returns, so a
  // single immediate re-scan can miss it — we bump now and a couple more times.
  const recentRunsTimersRef = useRef<ReturnType<typeof setTimeout>[]>([]);
  useEffect(
    () => () => {
      recentRunsTimersRef.current.forEach(clearTimeout);
      recentRunsTimersRef.current = [];
    },
    []
  );
  const scheduleRecentRunsRefresh = useCallback(() => {
    setRecentRunsRefreshNonce(n => n + 1);
    recentRunsTimersRef.current.forEach(clearTimeout);
    recentRunsTimersRef.current = [1500, 4000].map(ms =>
      setTimeout(() => setRecentRunsRefreshNonce(n => n + 1), ms)
    );
  }, []);
  // Guards against a fast double-click spawning two runs. `runSubmitGuardRef`
  // is the synchronous gate (a ref, so two clicks in the same tick both see
  // it); `runBusy` mirrors it into render so the buttons disable. After a
  // successful spawn we hold the guard for a short cooldown — the run-spawn
  // RPC returns in ~5ms, so without it a second click ~0.5s later (or before
  // the new run shows in Recent runs) would spawn a duplicate.
  const runSubmitGuardRef = useRef(false);
  const [runBusy, setRunBusy] = useState(false);
  const runCooldownTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(
    () => () => {
      if (runCooldownTimerRef.current) clearTimeout(runCooldownTimerRef.current);
    },
    []
  );
  // Release immediately (error → allow retry) or after a cooldown (success →
  // block accidental duplicates while the new run surfaces).
  const releaseRunGuard = useCallback((cooldownMs: number) => {
    if (runCooldownTimerRef.current) clearTimeout(runCooldownTimerRef.current);
    if (cooldownMs <= 0) {
      runSubmitGuardRef.current = false;
      setRunBusy(false);
      return;
    }
    runCooldownTimerRef.current = setTimeout(() => {
      runSubmitGuardRef.current = false;
      setRunBusy(false);
    }, cooldownMs);
  }, []);

  // Inline log viewer: one row expanded at a time. The viewer state map
  // is keyed by run_id so we keep paginated state per run without
  // refetching when the user collapses-and-re-expands the same row.
  const [expandedRunId, setExpandedRunId] = useState<string | null>(null);
  const [viewer, setViewer] = useState<
    Record<
      string,
      { content: string; offset: number; complete: boolean; loading: boolean; error: string | null }
    >
  >({});

  // Mirror of `viewer` into a ref so the tail-poll interval (whose effect
  // intentionally omits `viewer` from its deps) can read the *freshest*
  // offset/complete on each tick instead of the value captured when the
  // effect first ran. Without this the interval reuses a stale offset and
  // re-appends slices it already fetched (duplicate log output).
  const viewerRef = useRef(viewer);
  useEffect(() => {
    viewerRef.current = viewer;
  }, [viewer]);

  // ── Keep URL ?workflow= in sync with the picker ──────────────────────
  // Two-way binding so a manual picker change is reflected in the URL
  // (refresh-stable, back-button-friendly, shareable). `replace: true`
  // avoids stacking a history entry on every dropdown change. We only
  // touch the search-params when the value actually drifted to keep
  // React Router's effect bookkeeping quiet.
  useEffect(() => {
    const current = searchParams.get('workflow') ?? '';
    if (current === selectedSkillId) return;
    const next = new URLSearchParams(searchParams);
    if (selectedSkillId) {
      next.set('workflow', selectedSkillId);
    } else {
      next.delete('workflow');
    }
    setSearchParams(next, { replace: true });
  }, [selectedSkillId, searchParams, setSearchParams]);

  // ── React to URL changes (e.g. back/forward nav) ──────────────────
  // If the URL skill param drifts from the picker (back/forward, or
  // a programmatic navigate from elsewhere), follow the URL.
  useEffect(() => {
    const urlSkillId = searchParams.get('workflow') ?? '';
    if (urlSkillId !== selectedSkillId) {
      log('URL drift detected: url=%s picker=%s — following URL', urlSkillId, selectedSkillId);
      setSelectedSkillId(urlSkillId);
    }
    // Only re-run when the URL changes; selectedSkillId is the read of
    // the other side of the binding and is handled by the sync effect.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchParams]);

  // Deep-link from the workflow detail's "Schedule" button (`&focus=schedule`):
  // scroll the recurring-schedule section into view once the selected
  // workflow's body has rendered. rAF defers past the render commit.
  useEffect(() => {
    if (searchParams.get('focus') !== 'schedule' || !selectedSkillId) return;
    const timer = setTimeout(() => {
      document
        .getElementById('workflow-schedule')
        ?.scrollIntoView({ behavior: 'smooth', block: 'start' });
    }, 0);
    return () => clearTimeout(timer);
  }, [searchParams, selectedSkillId]);

  // ── Initial load: workflows_list ──────────────────────────────────────
  useEffect(() => {
    let cancelled = false;
    setSkillsLoading(true);
    setSkillsError(null);
    workflowsApi
      // Include `skills/`-root installs so registry-installed skills are runnable here.
      .listWorkflows({ includeSkills: true })
      .then(list => {
        if (cancelled) return;
        // Hide the codegraph-smoke skill — internal smoke-test only.
        const filtered = list.filter(s => s.id !== 'codegraph-smoke');
        setSkills(filtered);
        log('loaded %d skills', filtered.length);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        const msg = err instanceof Error ? err.message : String(err);
        log('listWorkflows error: %s', msg);
        setSkillsError(msg);
      })
      .finally(() => {
        if (!cancelled) setSkillsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // ── On selection: workflows_describe ──────────────────────────────────
  useEffect(() => {
    if (!selectedSkillId) {
      setDescription(null);
      setFormValues({});
      return;
    }
    let cancelled = false;
    setDescLoading(true);
    setDescError(null);
    setRun({ status: 'idle' });
    workflowsApi
      .describeWorkflow(selectedSkillId)
      .then(desc => {
        if (cancelled) return;
        setDescription(desc);
        // Seed form values from each input's default.
        const seed: Record<string, InputValue> = {};
        for (const i of desc.inputs) {
          seed[i.name] = defaultForType(i.type);
        }
        setFormValues(seed);
        log('described %s — %d inputs', selectedSkillId, desc.inputs.length);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        const msg = err instanceof Error ? err.message : String(err);
        log('describeWorkflow error: %s', msg);
        setDescError(msg);
      })
      .finally(() => {
        if (!cancelled) setDescLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedSkillId]);

  // ── Required-field validity ────────────────────────────────────────
  const missingRequired = useMemo(() => {
    if (!description) return [];
    const missing: string[] = [];
    for (const inp of description.inputs) {
      if (!inp.required) continue;
      const v = formValues[inp.name];
      if (v === undefined || v === null) {
        missing.push(inp.name);
        continue;
      }
      if (inp.type === 'boolean') continue; // false is a valid choice
      if (typeof v === 'string' && v.trim() === '') {
        missing.push(inp.name);
      }
    }
    return missing;
  }, [description, formValues]);

  // ── Run handler ────────────────────────────────────────────────────
  // Stop a RUNNING recent-run row via skill_runtime_cancel, then refresh the list
  // so it flips to CANCELLED.
  const handleStopRun = useCallback(async (runId: string) => {
    log('stop run runId=%s', runId);
    try {
      await workflowsApi.cancelRun(runId);
    } catch (err) {
      log('cancelRun error: %s', err instanceof Error ? err.message : String(err));
    }
    setRecentRunsRefreshNonce(n => n + 1);
  }, []);

  const ensureRuntimeAvailability = useCallback(async () => {
    const runtimeRequirement = inferRuntimeRequirement(selectedWorkflow);
    if (!runtimeRequirement) return;

    const resolved = await workflowsApi.resolveRuntimes(runtimeRequirement);
    const unavailable = resolved.runtimes.filter(runtime => !runtime.available);
    if (unavailable.length === 0) return;

    const prefix = t('settings.skillsRunner.error.runtimeUnavailable', 'Runtime unavailable');
    const defaultReason = t('settings.skillsRunner.error.runtimeUnavailableDefault', 'unavailable');
    throw new Error(
      unavailable
        .map(runtime => `${prefix}: ${runtime.runtime} (${runtime.error ?? defaultReason})`)
        .join('; ')
    );
  }, [selectedWorkflow, t]);

  const handleRun = useCallback(async () => {
    if (!description) return;
    // Re-entry guard: a second click before React applies the disabled state
    // would otherwise fire `skill_runtime_run` twice and spawn two real runs.
    if (runSubmitGuardRef.current) {
      log('runWorkflow: ignoring re-entrant click while a run is starting');
      return;
    }
    if (missingRequired.length > 0) {
      setRun({
        status: 'error',
        message: `${t('settings.skillsRunner.error.missingRequired')} ${missingRequired.join(', ')}`,
      });
      return;
    }
    runSubmitGuardRef.current = true;
    setRunBusy(true);
    setRun({ status: 'submitting' });
    try {
      const inputs = buildInputsPayload(description, formValues);
      log('runWorkflow %s inputs=%o', description.id, inputs);
      await ensureRuntimeAvailability();
      const result = await workflowsApi.runWorkflow(description.id, inputs);
      setRun({ status: 'started', result });
      // Surface the new run in "Recent runs" without a manual refresh, and
      // hold the guard through a short cooldown so a second click can't spawn
      // a duplicate before the run shows up.
      scheduleRecentRunsRefresh();
      releaseRunGuard(2500);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      log('runWorkflow error: %s', msg);
      setRun({ status: 'error', message: msg });
      releaseRunGuard(0); // allow immediate retry on failure
    }
  }, [
    description,
    formValues,
    missingRequired,
    scheduleRecentRunsRefresh,
    releaseRunGuard,
    ensureRuntimeAvailability,
    t,
  ]);

  // ── Recent runs: load on mount + on skill change + on demand ───────
  useEffect(() => {
    let cancelled = false;
    setRecentRunsLoading(true);
    workflowsApi
      .recentRuns(selectedSkillId || undefined, 10)
      .then(list => {
        if (cancelled) return;
        setRecentRuns(list);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        log('recentRuns error: %s', err instanceof Error ? err.message : String(err));
        setRecentRuns([]);
      })
      .finally(() => {
        if (!cancelled) setRecentRunsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedSkillId, recentRunsRefreshNonce]);

  // ── Scheduled jobs: load on skill change ───────────────────────────
  const loadScheduledJobs = useCallback(async () => {
    if (!selectedSkillId) {
      setScheduledJobs([]);
      return;
    }
    setScheduledJobsLoading(true);
    try {
      const resp = await openhumanCronList();
      const allJobs = (resp.result ?? []) as CoreCronJob[];
      const wanted = `${CRON_NAME_PREFIX}${selectedSkillId}`;
      // For the special dev-workflow skill, also surface legacy crons
      // saved by DevWorkflowPanel (named `dev-workflow-<repo>`) so the
      // user can toggle / edit them from the unified runner.
      const isDevWorkflow = selectedSkillId === 'dev-workflow';
      setScheduledJobs(
        allJobs.filter(j => {
          const n = j.name ?? '';
          if (n.startsWith(wanted)) return true;
          if (isDevWorkflow && n.startsWith('dev-workflow-')) return true;
          return false;
        })
      );
    } catch (err: unknown) {
      log('loadScheduledJobs error: %s', err instanceof Error ? err.message : String(err));
      setScheduledJobs([]);
    } finally {
      setScheduledJobsLoading(false);
    }
  }, [selectedSkillId]);

  useEffect(() => {
    void loadScheduledJobs();
  }, [loadScheduledJobs]);

  // ── Save schedule handler ──────────────────────────────────────────
  const handleSaveSchedule = useCallback(async () => {
    if (!description) return;
    if (missingRequired.length > 0) {
      setScheduleError(
        `${t('settings.skillsRunner.error.missingRequired')} ${missingRequired.join(', ')}`
      );
      return;
    }
    setSavingSchedule(true);
    setScheduleError(null);
    setScheduleSaved(false);
    try {
      const inputs = buildInputsPayload(description, formValues);
      const name = buildCronJobName(description.id, inputs);
      const prompt = buildAgentPrompt(description.id, inputs);
      log('saveSchedule name=%s schedule=%s', name, schedule);
      await openhumanCronAdd({
        name,
        schedule: { kind: 'cron', expr: schedule },
        job_type: 'agent',
        prompt,
        session_target: 'isolated',
        delivery: { mode: 'proactive', best_effort: true },
      });
      setScheduleSaved(true);
      if (scheduleSavedTimer.current) clearTimeout(scheduleSavedTimer.current);
      scheduleSavedTimer.current = setTimeout(() => setScheduleSaved(false), 3000);
      await loadScheduledJobs();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      log('saveSchedule error: %s', msg);
      setScheduleError(msg);
    } finally {
      setSavingSchedule(false);
    }
  }, [description, formValues, missingRequired, schedule, t, loadScheduledJobs]);

  // ── Log viewer: fetch on expand + tail-poll while running ──────────
  useEffect(() => {
    if (!expandedRunId) return;
    let cancelled = false;
    const runId = expandedRunId;

    // If we already loaded the full file and it's complete, don't refetch
    // — the user might just be re-expanding the same row. Read through the
    // ref so this reflects the latest state, not a stale effect closure.
    const existing = viewerRef.current[runId];
    if (existing?.complete) return;

    const fetchSlice = async (fromOffset: number): Promise<void> => {
      try {
        setViewer(prev => ({
          ...prev,
          [runId]: {
            content: prev[runId]?.content ?? '',
            offset: prev[runId]?.offset ?? 0,
            complete: prev[runId]?.complete ?? false,
            loading: true,
            error: null,
          },
        }));
        const slice: RunLogSlice = await workflowsApi.readRunLog(runId, fromOffset);
        if (cancelled) return;
        setViewer(prev => {
          const prior = prev[runId]?.content ?? '';
          return {
            ...prev,
            [runId]: {
              content: prior + slice.content,
              offset: slice.offset,
              complete: slice.complete,
              loading: false,
              error: null,
            },
          };
        });
      } catch (err: unknown) {
        if (cancelled) return;
        const msg = err instanceof Error ? err.message : String(err);
        log('readRunLog error: %s', msg);
        setViewer(prev => ({
          ...prev,
          [runId]: {
            content: prev[runId]?.content ?? '',
            offset: prev[runId]?.offset ?? 0,
            complete: prev[runId]?.complete ?? false,
            loading: false,
            error: msg,
          },
        }));
      }
    };

    // Initial fetch from where we left off (0 on first open).
    const startOffset = existing?.offset ?? 0;
    void fetchSlice(startOffset);

    // Tail every 2s while the run isn't complete. Re-reads the freshest
    // offset from state on each tick by ref-closure through fetchSlice.
    const interval = setInterval(() => {
      const state = viewerRef.current[runId];
      if (cancelled || state?.complete) {
        clearInterval(interval);
        return;
      }
      void fetchSlice(state?.offset ?? startOffset);
    }, 2000);

    return () => {
      cancelled = true;
      clearInterval(interval);
    };
    // We intentionally don't depend on `viewer` here — the interval reads
    // the freshest offset from `viewerRef.current` each tick, and re-running
    // this effect on every viewer update would tear down and re-create the
    // timer on every poll. Equally, depending on `viewer` would cause
    // an infinite re-render loop because setViewer happens inside.
  }, [expandedRunId]);

  const toggleExpand = useCallback((runId: string) => {
    setExpandedRunId(prev => (prev === runId ? null : runId));
  }, []);

  // ── Schedule-row actions ───────────────────────────────────────────
  // "Run" on a saved schedule: run the workflow NOW with that schedule's own
  // snapshotted inputs, via the same direct path as Run-now. (The cron tick
  // runs it through an agent prompt; triggering that here gave no visible run
  // and silently errored "already running" on repeat clicks — so run directly,
  // which spawns a real run that shows in Recent runs immediately.)
  const handleRunJobNow = useCallback(
    async (job: CoreCronJob) => {
      if (runSubmitGuardRef.current) {
        log('runJobNow: ignoring re-entrant click while a run is starting');
        return;
      }
      runSubmitGuardRef.current = true;
      setRunBusy(true);
      setScheduleError(null);
      const inputs = Object.fromEntries(
        parseScheduledInputs(job.prompt).map(i => [i.key, i.value])
      );
      try {
        log('runJobNow: running %s directly with %o', selectedSkillId, inputs);
        await ensureRuntimeAvailability();
        await workflowsApi.runWorkflow(selectedSkillId, inputs);
        scheduleRecentRunsRefresh();
        releaseRunGuard(2500);
      } catch (err: unknown) {
        const msg = err instanceof Error ? err.message : String(err);
        log('runJobNow error: %s', msg);
        setScheduleError(msg);
        releaseRunGuard(0);
      }
    },
    [selectedSkillId, scheduleRecentRunsRefresh, releaseRunGuard, ensureRuntimeAvailability]
  );

  const handleRemoveJob = useCallback(
    async (jobId: string) => {
      try {
        await openhumanCronRemove(jobId);
        await loadScheduledJobs();
      } catch (err: unknown) {
        log('removeJob error: %s', err instanceof Error ? err.message : String(err));
      }
    },
    [loadScheduledJobs]
  );

  // Mirror DevWorkflowPanel:439 — flip `enabled` and refresh the list.
  // We intentionally keep this generic on `job_id` so it works for any
  // skill, not just dev-workflow.
  const handleToggleJob = useCallback(
    async (job: CoreCronJob) => {
      try {
        await openhumanCronUpdate(job.id, { enabled: !job.enabled });
        await loadScheduledJobs();
      } catch (err: unknown) {
        log('toggleJob error: %s', err instanceof Error ? err.message : String(err));
      }
    },
    [loadScheduledJobs]
  );

  // ── Per-job history fetch ──────────────────────────────────────────
  // Ports DevWorkflowPanel:306-322 (loadRunHistory). The structured
  // cron `runs` list complements the cross-skill "Recent runs" panel
  // at the bottom of the body, which scans skill_run log files; here
  // we get authoritative cron-run records keyed off the specific
  // schedule (status / duration / output stored at tick time).
  const loadJobHistory = useCallback(async (jobId: string) => {
    setHistoryState(prev => ({
      ...prev,
      [jobId]: {
        runs: prev[jobId]?.runs ?? [],
        loading: true,
        expanded: true,
        expandedRunId: prev[jobId]?.expandedRunId ?? null,
      },
    }));
    try {
      const res = await openhumanCronRuns(jobId, 5);
      const raw = (res as { result?: { runs?: CoreCronRun[] } | CoreCronRun[] }).result;
      const runs = Array.isArray(raw) ? raw : (raw?.runs ?? []);
      setHistoryState(prev => ({
        ...prev,
        [jobId]: {
          runs: Array.isArray(runs) ? runs : [],
          loading: false,
          expanded: true,
          expandedRunId: prev[jobId]?.expandedRunId ?? null,
        },
      }));
      log('loaded %d history entries for job %s', Array.isArray(runs) ? runs.length : 0, jobId);
    } catch (err: unknown) {
      log('loadJobHistory error: %s', err instanceof Error ? err.message : String(err));
      setHistoryState(prev => ({
        ...prev,
        [jobId]: {
          runs: prev[jobId]?.runs ?? [],
          loading: false,
          expanded: true,
          expandedRunId: prev[jobId]?.expandedRunId ?? null,
        },
      }));
    }
  }, []);

  const toggleJobHistory = useCallback(
    (jobId: string) => {
      setHistoryState(prev => {
        const cur = prev[jobId];
        if (cur?.expanded) {
          return { ...prev, [jobId]: { ...cur, expanded: false } };
        }
        return prev;
      });
      const cur = historyState[jobId];
      if (!cur?.expanded) {
        void loadJobHistory(jobId);
      }
    },
    [historyState, loadJobHistory]
  );

  const toggleHistoryRun = useCallback((jobId: string, runId: number) => {
    setHistoryState(prev => {
      const cur = prev[jobId];
      if (!cur) return prev;
      return {
        ...prev,
        [jobId]: { ...cur, expandedRunId: cur.expandedRunId === runId ? null : runId },
      };
    });
  }, []);

  // ── Form-field renderer ────────────────────────────────────────────
  // Convention-based rich pickers: if the input's name is one of the
  // repo/branch conventional names, render a Composio-backed picker
  // instead of a plain text input. Falls through to the type-based
  // string/integer/boolean handling for everything else.
  const renderField = (
    inp: WorkflowDescription['inputs'][number],
    value: InputValue,
    onChange: (next: InputValue) => void
  ) => {
    const id = `skills-runner-input-${inp.name}`;
    const requiredMark = inp.required ? <span className="text-red-500"> *</span> : null;
    const commonLabel = (
      <label
        htmlFor={id}
        className="block text-sm font-medium text-content-secondary dark:text-stone-300 mb-1">
        {inp.name}
        {requiredMark}
      </label>
    );
    const desc = inp.description ? (
      <p className="text-xs text-content-muted dark:text-stone-400 mt-1">{inp.description}</p>
    ) : null;

    // Rich picker: repo-shaped input → Composio github_repo picker.
    if (REPO_INPUT_NAMES.has(inp.name)) {
      return (
        <div key={inp.name}>
          {commonLabel}
          <RepoPicker id={id} value={typeof value === 'string' ? value : ''} onChange={onChange} />
          {desc}
        </div>
      );
    }
    // Rich picker: branch-shaped input → branch dropdown, depends on
    // the resolved sibling repo-shaped input value.
    if (BRANCH_INPUT_NAMES.has(inp.name)) {
      const linkedRepo = resolveLinkedRepo(formValues);
      return (
        <div key={inp.name}>
          {commonLabel}
          <BranchPicker
            id={id}
            value={typeof value === 'string' ? value : ''}
            onChange={onChange}
            repo={linkedRepo}
          />
          {desc}
        </div>
      );
    }

    if (inp.type === 'boolean') {
      return (
        <div key={inp.name}>
          <label
            htmlFor={id}
            className="flex items-center gap-2 text-sm font-medium text-content-secondary dark:text-stone-300">
            <input
              id={id}
              type="checkbox"
              checked={Boolean(value)}
              onChange={e => onChange(e.target.checked)}
              className="rounded"
            />
            {inp.name}
            {requiredMark}
          </label>
          {desc}
        </div>
      );
    }

    if (inp.type === 'integer') {
      return (
        <div key={inp.name}>
          {commonLabel}
          <input
            id={id}
            type="number"
            inputMode="numeric"
            value={typeof value === 'number' ? value : (value as string)}
            onChange={e => onChange(e.target.value)}
            placeholder={inp.required ? t('settings.skillsRunner.placeholder.required') : ''}
            className="w-full rounded border border-line-strong dark:border-stone-600 bg-surface px-3 py-2 text-sm text-content dark:text-stone-100"
          />
          {desc}
        </div>
      );
    }

    // string (default)
    return (
      <div key={inp.name}>
        {commonLabel}
        <input
          id={id}
          type="text"
          value={value as string}
          onChange={e => onChange(e.target.value)}
          placeholder={inp.required ? t('settings.skillsRunner.placeholder.required') : ''}
          className="w-full rounded border border-line-strong dark:border-stone-600 bg-surface px-3 py-2 text-sm text-content dark:text-stone-100"
        />
        {desc}
      </div>
    );
  };

  // ── Render ─────────────────────────────────────────────────────────
  return (
    <div className={className ?? 'space-y-6'}>
      <div className="text-sm text-content-secondary dark:text-stone-400">
        {headerText ?? t('settings.developerMenu.skillsRunner.panelDesc')}
      </div>

      {/* Workflow header (locked) / picker (embedded). When the page is
            locked to one workflow, the name is the page heading with Edit
            beside it — not a disabled-looking form field. */}
      <div>
        {locked ? (
          <div className="flex items-center gap-2">
            <h2
              data-testid="skills-runner-skill-locked"
              className="min-w-0 truncate text-lg font-semibold text-content dark:text-stone-100">
              {selectedWorkflow?.name || selectedSkillId}
            </h2>
          </div>
        ) : (
          <>
            <label
              htmlFor="skills-runner-skill"
              className="block text-sm font-medium text-content-secondary dark:text-stone-300 mb-1">
              {t('settings.skillsRunner.skill')}
            </label>
            <select
              id="skills-runner-skill"
              value={selectedSkillId}
              onChange={e => setSelectedSkillId(e.target.value)}
              disabled={skillsLoading || skillsError !== null}
              className="w-full rounded border border-line-strong dark:border-stone-600 bg-surface px-3 py-2 text-sm text-content dark:text-stone-100">
              <option value="">
                {skillsLoading
                  ? t('settings.skillsRunner.loadingSkills')
                  : t('settings.skillsRunner.selectSkill')}
              </option>
              {skills.map(s => (
                <option key={s.id} value={s.id}>
                  {s.name || s.id}
                </option>
              ))}
            </select>
          </>
        )}
        {skillsError && (
          <p className="text-xs text-red-600 dark:text-red-400 mt-1">
            {t('settings.skillsRunner.error.listWorkflows')} {skillsError}
          </p>
        )}
      </div>

      {/* Description + form */}
      {selectedSkillId && (
        <>
          {descLoading && (
            <div className="text-sm text-content-muted dark:text-stone-400">
              {t('settings.skillsRunner.loadingDescription')}
            </div>
          )}
          {descError && (
            <div className="text-sm text-red-600 dark:text-red-400">
              {t('settings.skillsRunner.error.describe')} {descError}
            </div>
          )}
          {description && (
            <>
              {/* Description + Inputs + Run + Schedule in one box — read
                    what it does, fill the inputs once, then either Run now or
                    save a recurring schedule that snapshots them. */}
              <div
                id="workflow-schedule"
                className="space-y-4 rounded-2xl border border-line/90 dark:border-stone-700/80 bg-gradient-to-br from-stone-50 via-white to-stone-100 dark:from-stone-900 dark:via-stone-900 dark:to-stone-800/80 px-4 py-4 shadow-soft">
                <div className="rounded border border-line dark:border-stone-700 bg-surface/70 dark:bg-stone-900/60 p-3">
                  <p className="text-sm text-content-secondary dark:text-stone-300 whitespace-pre-wrap">
                    {description.when_to_use}
                  </p>
                </div>

                {description.inputs.length === 0 ? (
                  <p className="text-sm italic text-content-muted dark:text-stone-400">
                    {t('settings.skillsRunner.noInputs')}
                  </p>
                ) : (
                  <div className="space-y-4">
                    {SMART_PICKER_SKILL_IDS.has(description.id) && (
                      <SmartIssuePicker
                        values={{
                          repo:
                            typeof formValues.repo === 'string' ? (formValues.repo as string) : '',
                          upstream:
                            typeof formValues.upstream === 'string'
                              ? (formValues.upstream as string)
                              : '',
                          target_branch:
                            typeof formValues.target_branch === 'string'
                              ? (formValues.target_branch as string)
                              : '',
                          fork_owner:
                            typeof formValues.fork_owner === 'string'
                              ? (formValues.fork_owner as string)
                              : '',
                        }}
                        onPatchInputs={patch => setFormValues(prev => ({ ...prev, ...patch }))}
                      />
                    )}
                    {description.inputs
                      .filter(inp => {
                        // When the smart picker is mounted, hide the
                        // inputs it manages — the picker already drives
                        // them via onPatchInputs and the user shouldn't
                        // see duplicate raw text fields for the same
                        // values. Other (future) inputs render as
                        // normal.
                        if (
                          SMART_PICKER_SKILL_IDS.has(description.id) &&
                          SMART_PICKER_INPUT_NAMES.has(inp.name)
                        ) {
                          return false;
                        }
                        return true;
                      })
                      .map(inp =>
                        renderField(inp, formValues[inp.name] ?? defaultForType(inp.type), next =>
                          setFormValues(prev => ({ ...prev, [inp.name]: next }))
                        )
                      )}
                  </div>
                )}

                {/* Run now + Edit — act on this workflow with the inputs above. */}
                <div className="flex flex-col gap-2">
                  <div className="flex flex-wrap items-center gap-2">
                    <Button
                      variant="primary"
                      onClick={() => void handleRun()}
                      disabled={
                        run.status === 'submitting' || runBusy || missingRequired.length > 0
                      }>
                      {run.status === 'submitting'
                        ? t('settings.skillsRunner.starting')
                        : t('settings.skillsRunner.runNow')}
                    </Button>
                    {selectedWorkflow &&
                      selectedWorkflow.scope === 'user' &&
                      !selectedWorkflow.legacy && (
                        <Button
                          variant="secondary"
                          data-testid="skills-runner-edit"
                          onClick={() => setEditOpen(true)}
                          leadingIcon={<span aria-hidden="true">✎</span>}>
                          {t('common.edit')}
                        </Button>
                      )}
                  </div>

                  {run.status === 'started' && run.result && (
                    <div className="rounded border border-emerald-300 dark:border-emerald-700 bg-emerald-50 dark:bg-emerald-950 p-3 text-sm">
                      <p className="text-emerald-800 dark:text-emerald-200">
                        {t('settings.skillsRunner.started')} {run.result.run_id}
                      </p>
                      <p className="text-xs text-emerald-700 dark:text-emerald-300 mt-1 break-all">
                        {t('settings.skillsRunner.logPath')} <code>{run.result.log}</code>
                      </p>
                    </div>
                  )}
                  {run.status === 'error' &&
                    (() => {
                      // Detect the `[preflight:<gate>:<tag>] <body>` shape
                      // emitted by spawn_skill_run_background's preflight
                      // branch (src/openhuman/skills/preflight.rs). When
                      // matched, surface a dedicated "Preflight gate
                      // failed" pill above the body so the user knows
                      // this isn't a generic crash — there's a concrete
                      // remediation the body describes.
                      const parsed = parseWorkflowRunError(run.message);
                      const isGateFailure = isGithubGateFailure(parsed);
                      return (
                        <div
                          data-testid="skill-run-error"
                          className="rounded border border-red-300 dark:border-red-700 bg-red-50 dark:bg-red-950 p-3 text-sm">
                          {isGateFailure && (
                            <div
                              data-testid="preflight-gate-pill"
                              className="mb-1.5 inline-flex items-center gap-1 rounded-full bg-amber-100 dark:bg-amber-900 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-amber-800 dark:text-amber-200">
                              {t('settings.skillsRunner.error.preflightGate')}
                              {parsed.tag ? (
                                <code className="font-mono text-[10px] opacity-80">
                                  {parsed.tag}
                                </code>
                              ) : null}
                            </div>
                          )}
                          <p className="text-red-800 dark:text-red-200">
                            {isGateFailure
                              ? parsed.body
                              : `${t('settings.skillsRunner.error.run')} ${run.message ?? ''}`}
                          </p>
                        </div>
                      );
                    })()}
                </div>

                {/* Same inputs, second action: run it on a schedule. */}
                <div className="border-t border-line/70 dark:border-stone-700/70 pt-3">
                  <h3 className="mb-2 text-sm font-semibold text-content dark:text-stone-200">
                    {t('settings.skillsRunner.schedule.heading')}
                  </h3>
                  <div className="flex flex-col sm:flex-row sm:items-end gap-2">
                    <div className="flex-1">
                      <label
                        htmlFor="skills-runner-schedule"
                        className="block text-xs font-semibold uppercase tracking-wide text-content-secondary dark:text-stone-300 mb-1.5">
                        {t('settings.skillsRunner.schedule.frequency')}
                      </label>
                      <select
                        id="skills-runner-schedule"
                        value={schedule}
                        onChange={e => setSchedule(e.target.value)}
                        className="w-full rounded-xl border border-line-strong dark:border-stone-600 bg-surface px-3 py-2 text-sm text-content dark:text-stone-100 shadow-sm focus:outline-none focus:ring-2 focus:ring-primary-500/40 focus:border-primary-500">
                        {SCHEDULE_PRESETS.map(p => (
                          <option key={p.value} value={p.value}>
                            {t(p.labelKey)}
                          </option>
                        ))}
                      </select>
                    </div>
                    <Button
                      variant="primary"
                      onClick={() => void handleSaveSchedule()}
                      disabled={savingSchedule || missingRequired.length > 0}
                      className="shadow-soft">
                      {savingSchedule
                        ? t('settings.skillsRunner.schedule.saving')
                        : t('settings.skillsRunner.schedule.save')}
                    </Button>
                  </div>

                  {scheduleSaved && (
                    <p className="mt-2 inline-flex items-center rounded-full border border-emerald-300 dark:border-emerald-700 bg-emerald-50 dark:bg-emerald-900/40 px-2.5 py-1 text-xs font-medium text-emerald-700 dark:text-emerald-300">
                      {t('settings.skillsRunner.schedule.saved')}
                    </p>
                  )}
                  {scheduleError && (
                    <p className="mt-2 inline-flex items-center rounded-full border border-red-300 dark:border-red-700 bg-red-50 dark:bg-red-900/40 px-2.5 py-1 text-xs font-medium text-red-700 dark:text-red-300">
                      {t('settings.skillsRunner.schedule.error')} {scheduleError}
                    </p>
                  )}
                </div>
              </div>

              {/* Second box — saved schedules for this workflow. */}
              <div className="space-y-2 rounded-2xl border border-line/90 dark:border-stone-700/80 bg-surface dark:bg-stone-900 px-4 py-4 shadow-soft">
                {/* Existing scheduled jobs for this skill */}
                {scheduledJobsLoading ? (
                  <p className="mt-3 text-xs text-content-muted dark:text-stone-400">
                    {t('settings.skillsRunner.schedule.loadingJobs')}
                  </p>
                ) : scheduledJobs.length === 0 ? (
                  <p className="mt-3 text-xs italic text-content-muted dark:text-stone-400">
                    {t('settings.skillsRunner.schedule.noJobs')}
                  </p>
                ) : (
                  <div className="mt-3 space-y-2 rounded-2xl border border-line/80 dark:border-stone-800 bg-surface-muted/70 dark:bg-stone-900/40 p-2.5">
                    <div className="text-xs font-semibold uppercase tracking-wide text-content-secondary dark:text-stone-400 px-1">
                      {t('settings.skillsRunner.schedule.existing')}
                    </div>
                    {/* Per-skill saved-schedule list — uses the shared
                          ScheduledCronCard so the runner and the global
                          /skills dashboard render the same polished card
                          chrome (toggle + cronToHuman + last/next run).
                          Run-Now / Remove live in the card's `actions`
                          slot; the lazy per-job history disclosure
                          lives in `children`. The card emits testids
                          `scheduled-job-<id>` (root) and
                          `scheduled-job-<id>-toggle` (switch); the
                          history pieces below keep their own testids
                          (`history-toggle-<id>`, `history-run-<id>-<runId>`). */}
                    {sortedScheduledJobs.map(job => {
                      const hist = historyState[job.id];
                      const isActive = job.id === activeJobId;
                      // Each job's own inputs, recovered from the prompt it
                      // was created with.
                      const jobInputs = parseScheduledInputs(job.prompt);
                      return (
                        <ScheduledCronCard
                          key={job.id}
                          job={job}
                          title={job.name ?? job.id}
                          activeBadge={isActive}
                          onToggle={() => void handleToggleJob(job)}
                          testIdRoot={`scheduled-job-${job.id}`}
                          actions={
                            <>
                              <Button
                                variant="primary"
                                size="sm"
                                disabled={runBusy}
                                onClick={() => void handleRunJobNow(job)}>
                                {t('settings.skillsRunner.schedule.runNow')}
                              </Button>
                              <Button
                                variant="primary"
                                tone="danger"
                                size="sm"
                                onClick={() => void handleRemoveJob(job.id)}>
                                {t('settings.skillsRunner.schedule.remove')}
                              </Button>
                            </>
                          }>
                          {/* The inputs this schedule was created with —
                                each job keeps its own snapshot, so the card
                                shows exactly what this schedule runs with. */}
                          <div data-testid={`scheduled-job-${job.id}-inputs`} className="px-4 pt-2">
                            {jobInputs.length > 0 ? (
                              <div className="flex flex-wrap items-center gap-1">
                                <span className="text-[10px] font-semibold uppercase tracking-wide text-content-muted dark:text-stone-400">
                                  {t('settings.skillsRunner.schedule.inputsLabel')}
                                </span>
                                {jobInputs.map(inp => (
                                  <span
                                    key={inp.key}
                                    className="rounded bg-surface-subtle dark:bg-stone-800 px-1.5 py-0.5 font-mono text-[10px] text-content-secondary dark:text-stone-300">
                                    {inp.key}: {inp.value}
                                  </span>
                                ))}
                              </div>
                            ) : (
                              <span className="text-[10px] italic text-content-faint dark:text-stone-500">
                                {t('settings.skillsRunner.schedule.inputsNone')}
                              </span>
                            )}
                          </div>
                          {/* Per-job run history (lazy on first expand).
                                Ports DevWorkflowPanel:591-645's pattern:
                                a disclosure toggle reveals up to 5 runs
                                each with status badge + duration; click
                                a run to expand its captured output. */}
                          <div className="px-4 pb-3 border-t border-line-subtle dark:border-stone-800">
                            <button
                              type="button"
                              onClick={() => toggleJobHistory(job.id)}
                              aria-expanded={Boolean(hist?.expanded)}
                              data-testid={`history-toggle-${job.id}`}
                              className="mt-2 text-[11px] text-content-secondary dark:text-stone-400 hover:underline">
                              {hist?.expanded ? '▾' : '▸'}{' '}
                              {t('settings.skillsRunner.schedule.history')}
                              {hist?.runs?.length ? ` (${hist.runs.length})` : ''}
                            </button>
                            {hist?.expanded && (
                              <div className="mt-1.5 space-y-1">
                                {hist.loading && hist.runs.length === 0 ? (
                                  <p className="text-[11px] text-content-muted dark:text-stone-400">
                                    {t('settings.skillsRunner.schedule.historyLoading')}
                                  </p>
                                ) : hist.runs.length === 0 ? (
                                  <p className="text-[11px] italic text-content-muted dark:text-stone-400">
                                    {t('settings.skillsRunner.schedule.historyEmpty')}
                                  </p>
                                ) : (
                                  hist.runs.map(r => {
                                    const open = hist.expandedRunId === r.id;
                                    const okClass =
                                      r.status === 'ok'
                                        ? 'bg-sage-100 dark:bg-sage-500/20 text-sage-700 dark:text-sage-300'
                                        : 'bg-coral-100 dark:bg-coral-500/20 text-coral-700 dark:text-coral-300';
                                    return (
                                      <div
                                        key={r.id}
                                        className="rounded bg-surface">
                                        <button
                                          type="button"
                                          onClick={() => toggleHistoryRun(job.id, r.id)}
                                          aria-expanded={open}
                                          data-testid={`history-run-${job.id}-${r.id}`}
                                          className="w-full flex items-center justify-between px-2 py-1.5 hover:bg-surface-muted dark:hover:bg-stone-700 rounded">
                                          <div className="flex items-center gap-2">
                                            <span className="text-content-faint">
                                              {open ? '▾' : '▸'}
                                            </span>
                                            <span className="text-content-secondary dark:text-stone-400">
                                              {new Date(r.started_at).toLocaleString()}
                                            </span>
                                          </div>
                                          <div className="flex items-center gap-2">
                                            {r.duration_ms != null && (
                                              <span className="text-content-muted">
                                                {(r.duration_ms / 1000).toFixed(1)}s
                                              </span>
                                            )}
                                            <span
                                              className={`px-1.5 py-0.5 rounded text-[10px] font-medium ${okClass}`}>
                                              {r.status}
                                            </span>
                                          </div>
                                        </button>
                                        {open && r.output && (
                                          <pre className="mx-2 mb-2 px-3 py-2 rounded-md bg-surface-subtle dark:bg-stone-900 border border-line dark:border-stone-700 text-[11px] text-content-secondary dark:text-stone-300 font-mono whitespace-pre-wrap break-words max-h-64 overflow-y-auto">
                                            {r.output}
                                          </pre>
                                        )}
                                        {open && !r.output && (
                                          <div className="mx-2 mb-2 px-3 py-2 text-[11px] italic text-content-faint dark:text-stone-500">
                                            {t('settings.skillsRunner.schedule.historyNoOutput')}
                                          </div>
                                        )}
                                      </div>
                                    );
                                  })
                                )}
                              </div>
                            )}
                          </div>
                        </ScheduledCronCard>
                      );
                    })}
                  </div>
                )}
              </div>
            </>
          )}
        </>
      )}

      {/* Recent runs (cross-skill if no skill picked; otherwise scoped) */}
      <div className="pt-4 border-t border-line dark:border-stone-700 space-y-2">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-content-secondary dark:text-stone-300">
            {selectedSkillId
              ? t('settings.skillsRunner.recentRuns.headingForSkill')
              : t('settings.skillsRunner.recentRuns.headingAll')}
          </h3>
          <Button
            variant="tertiary"
            size="xs"
            onClick={() => setRecentRunsRefreshNonce(n => n + 1)}
            className="px-0 text-content-secondary hover:bg-transparent hover:underline dark:text-stone-400">
            {t('settings.skillsRunner.recentRuns.refresh')}
          </Button>
        </div>
        {recentRunsLoading ? (
          <p className="text-xs text-content-muted dark:text-stone-400">
            {t('settings.skillsRunner.recentRuns.loading')}
          </p>
        ) : recentRuns.length === 0 ? (
          <p className="text-xs italic text-content-muted dark:text-stone-400">
            {t('settings.skillsRunner.recentRuns.empty')}
          </p>
        ) : (
          <div className="space-y-2">
            {recentRuns.map(r => {
              const badgeClass = (() => {
                if (r.status === 'RUNNING')
                  return 'bg-blue-100 text-blue-800 dark:bg-blue-900 dark:text-blue-200';
                if (r.status === 'DONE')
                  return 'bg-emerald-100 text-emerald-800 dark:bg-emerald-900 dark:text-emerald-200';
                if (r.status === 'DEGENERATE')
                  return 'bg-amber-100 text-amber-800 dark:bg-amber-900 dark:text-amber-200';
                return 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200';
              })();
              const dur = r.duration_ms !== null ? `${Math.round(r.duration_ms / 1000)}s` : '—';
              const expanded = expandedRunId === r.run_id;
              const v = viewer[r.run_id];
              return (
                <div
                  key={r.run_id}
                  className="rounded border border-line dark:border-stone-700 bg-surface-muted dark:bg-stone-900 text-xs overflow-hidden">
                  <button
                    type="button"
                    onClick={() => toggleExpand(r.run_id)}
                    className="w-full text-left px-3 py-2 hover:bg-surface-subtle dark:hover:bg-stone-800 focus:outline-none focus:bg-surface-subtle dark:focus:bg-stone-800"
                    aria-expanded={expanded}>
                    <div className="flex items-center gap-2 mb-1">
                      <span className="text-content-muted dark:text-stone-400">
                        {expanded ? '▾' : '▸'}
                      </span>
                      <span className={`px-1.5 py-0.5 rounded text-xs font-medium ${badgeClass}`}>
                        {r.status}
                      </span>
                      <span className="font-mono text-content-secondary dark:text-stone-300">
                        {r.run_id.slice(0, 8)}
                      </span>
                      <span className="text-content-secondary dark:text-stone-400">{r.workflow_id}</span>
                      <span className="text-content-muted dark:text-stone-400 ml-auto">{dur}</span>
                    </div>
                    <div className="text-content-muted dark:text-stone-400 truncate pl-5">
                      {r.started}
                    </div>
                    <div className="text-content-faint dark:text-stone-500 font-mono text-[10px] truncate pl-5">
                      {r.log_path}
                    </div>
                  </button>

                  {r.status === 'RUNNING' && (
                    <div className="px-3 pb-2 pl-8">
                      <Button
                        variant="secondary"
                        tone="danger"
                        size="xs"
                        data-testid={`run-stop-${r.run_id}`}
                        onClick={() => void handleStopRun(r.run_id)}
                        leadingIcon={<span aria-hidden="true">◼</span>}>
                        {t('autocomplete.stop')}
                      </Button>
                    </div>
                  )}

                  {expanded && (
                    <div className="border-t border-line dark:border-stone-700 bg-surface dark:bg-stone-950">
                      {/* Live indicator while tailing */}
                      {!v?.complete && (
                        <div className="px-3 py-1.5 text-[10px] text-content-muted dark:text-stone-400 border-b border-line-subtle dark:border-stone-800 flex items-center gap-2">
                          <span className="inline-block h-1.5 w-1.5 rounded-full bg-blue-500 animate-pulse" />
                          <span>
                            {t('settings.skillsRunner.viewer.tailing')}
                            {v?.loading ? ` · ${t('settings.skillsRunner.viewer.fetching')}` : ''}
                          </span>
                          <span className="ml-auto text-content-faint dark:text-stone-500">
                            {v?.offset ?? 0} B
                          </span>
                        </div>
                      )}
                      {v?.error && (
                        <div className="px-3 py-2 text-red-700 dark:text-red-300 bg-red-50 dark:bg-red-950 border-b border-red-100 dark:border-red-900">
                          {t('settings.skillsRunner.viewer.error')} {v.error}
                        </div>
                      )}
                      <pre className="px-3 py-2 m-0 max-h-96 overflow-auto font-mono text-[11px] leading-snug whitespace-pre-wrap break-words text-content dark:text-stone-200">
                        {v?.content ??
                          (v?.loading ? t('settings.skillsRunner.viewer.loading') : '')}
                      </pre>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* Edit-this-workflow modal (locked view only). Refreshes metadata +
          inputs on save. */}
      {editOpen && selectedWorkflow && (
        <CreateSkillModal
          editing={selectedWorkflow}
          onClose={() => setEditOpen(false)}
          onCreated={() => {
            setEditOpen(false);
            void workflowsApi
              .listWorkflows({ includeSkills: true })
              .then(list => setSkills(list.filter(s => s.id !== 'codegraph-smoke')))
              .catch(() => {});
            void workflowsApi
              .describeWorkflow(selectedSkillId)
              .then(setDescription)
              .catch(() => {});
          }}
        />
      )}
    </div>
  );
};

export default WorkflowRunnerBody;
