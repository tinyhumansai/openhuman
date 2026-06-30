import createDebug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { execute as composioExecute, listConnections } from '../../../lib/composio/composioApi';
import { SCHEDULE_PRESETS } from '../../../lib/cron/schedulePresets';
import { useT } from '../../../lib/i18n/I18nContext';
import {
  CoreCronJob,
  CoreCronRun,
  CronAddParams,
  openhumanCronAdd,
  openhumanCronList,
  openhumanCronRemove,
  openhumanCronRun,
  openhumanCronRuns,
  openhumanCronUpdate,
} from '../../../utils/tauriCommands/cron';
import Button from '../../ui/Button';
import {
  SettingsRow,
  SettingsSection,
  SettingsSelect,
  SettingsStatusLine,
  SettingsSwitch,
} from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

const log = createDebug('app:settings:DevWorkflowPanel');

// ── Types ──────────────────────────────────────────────────────────────

/** Shape returned by `openhuman.composio_list_github_repos`. */
interface ComposioGhRepo {
  owner: string;
  repo: string;
  fullName: string;
  private?: boolean;
  defaultBranch?: string;
  htmlUrl?: string;
}

interface ForkInfo {
  isFork: boolean;
  upstreamOwner: string;
  upstreamRepo: string;
  upstreamFullName: string;
}

interface GhBranch {
  name: string;
}

// ── Component ──────────────────────────────────────────────────────────

const DevWorkflowPanel = () => {
  const { t } = useT();

  // Repo list
  const [repos, setRepos] = useState<ComposioGhRepo[]>([]);
  const [reposLoading, setReposLoading] = useState(false);
  const [reposError, setReposError] = useState<string | null>(null);

  // Form state
  const [selectedRepo, setSelectedRepo] = useState('');
  const [forkInfo, setForkInfo] = useState<ForkInfo | null>(null);
  const [targetBranch, setTargetBranch] = useState('');
  const [schedule, setSchedule] = useState(SCHEDULE_PRESETS[0].value);

  // Fork detection loading
  const [forkLoading, setForkLoading] = useState(false);

  // Branches
  const [branches, setBranches] = useState<GhBranch[]>([]);
  const [branchesLoading, setBranchesLoading] = useState(false);

  // Save state
  const [saveStatus, setSaveStatus] = useState<'idle' | 'saved' | 'error'>('idle');

  // Cron job state
  const [existingJob, setExistingJob] = useState<CoreCronJob | null>(null);
  const [cronLoading, setCronLoading] = useState(false);
  const [runHistory, setRunHistory] = useState<CoreCronRun[]>([]);
  const [historyExpanded, setHistoryExpanded] = useState(false);
  const [expandedRunId, setExpandedRunId] = useState<number | null>(null);
  const [running, setRunning] = useState(false);

  // ── Load existing cron job on mount ─────────────────────────────────
  const loadExistingJob = useCallback(async () => {
    setCronLoading(true);
    try {
      const res = await openhumanCronList();
      // RPC returns { result: CronJob[], logs: [...] }
      const jobs = (res as { result?: CoreCronJob[] }).result ?? [];
      const jobList = Array.isArray(jobs) ? jobs : [];
      const found = jobList.find((j: CoreCronJob) => j.name?.startsWith('dev-workflow') ?? false);
      if (found) {
        setExistingJob(found);

        // Restore form state from the stored job so returning to the panel
        // with an active job doesn't show blank dropdowns.
        const restored: { repo?: string; schedule?: string; branch?: string } = {};

        // Schedule: prefer the structured cron expr, fall back to `expression`.
        const scheduleExpr =
          (found.schedule?.kind === 'cron' ? found.schedule.expr : undefined) ??
          found.expression ??
          undefined;
        if (scheduleExpr) {
          setSchedule(scheduleExpr);
          restored.schedule = scheduleExpr;
        }

        // Repo: encoded in the job name as `dev-workflow-<owner>-<repo>`
        // where the original `/` separator became the first `-` after the prefix.
        const repoSlug = found.name?.replace(/^dev-workflow-/, '') ?? '';
        if (repoSlug) {
          // Re-derive `owner/repo` by replacing only the first `-` with `/`.
          const fullName = repoSlug.replace('-', '/');
          setSelectedRepo(fullName);
          restored.repo = fullName;
        }

        // Target branch: recoverable from the prompt, which embeds
        // `PRs target \`<branch>\``.
        const branchMatch = found.prompt?.match(/PRs target `([^`]+)`/);
        if (branchMatch?.[1]) {
          setTargetBranch(branchMatch[1]);
          restored.branch = branchMatch[1];
        }

        log(
          'found existing dev-workflow cron job: %s, restored form state: %o',
          found.id,
          restored
        );
      } else {
        setExistingJob(null);
        log('no existing dev-workflow cron job found');
      }
    } catch (err) {
      log('failed to load existing cron job: %s', err);
    } finally {
      setCronLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadExistingJob();
  }, [loadExistingJob]);

  // ── Fetch repos via composio_execute ────────────────────────────────
  const loadRepos = useCallback(async () => {
    setReposLoading(true);
    setReposError(null);
    try {
      // Step 1: Check if GitHub is connected via Composio
      log('checking GitHub connection status');
      const connections = await listConnections();
      const ghConn = connections.connections?.find(
        c =>
          c.toolkit.toLowerCase().includes('github') &&
          (c.status === 'ACTIVE' || c.status === 'CONNECTED')
      );
      if (!ghConn) {
        throw new Error('NOT_CONNECTED');
      }
      log('GitHub connected, connectionId=%s', ghConn.id);

      // Step 2: Fetch repos via composio_execute
      log('fetching repos via GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER');
      const res = await composioExecute('GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER', {});
      if (!res.successful) {
        throw new Error(res.error ?? 'Failed to fetch repositories');
      }

      // Step 3: Parse response — GitHub API returns an array of repo objects
      const raw = res.data;
      let repoList: ComposioGhRepo[] = [];
      const items = Array.isArray(raw)
        ? raw
        : ((raw as Record<string, unknown>)?.repositories ?? []);
      if (Array.isArray(items)) {
        repoList = (items as Record<string, unknown>[]).map(r => ({
          owner: String((r.owner as Record<string, unknown>)?.login ?? r.owner ?? ''),
          repo: String(r.name ?? ''),
          fullName: String(
            r.full_name ?? `${(r.owner as Record<string, unknown>)?.login ?? r.owner}/${r.name}`
          ),
          private: r.private as boolean | undefined,
          defaultBranch: r.default_branch as string | undefined,
          htmlUrl: r.html_url as string | undefined,
        }));
      }

      log('fetched %d repos', repoList.length);
      setRepos(repoList);
      if (repoList.length === 0) {
        setReposError(t('settings.devWorkflow.errorNoRepositories'));
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('loadRepos error: %s', msg);
      if (msg === 'NOT_CONNECTED') {
        setReposError(t('settings.devWorkflow.errorNotConnected'));
      } else if (msg.includes('ToolNotFound') || msg.includes('not found')) {
        setReposError(t('settings.devWorkflow.errorToolNotEnabled'));
      } else if (
        msg.includes('session') ||
        msg.includes('composio unavailable') ||
        msg.includes('Sign in')
      ) {
        setReposError(t('settings.devWorkflow.errorNotAuthenticated'));
      } else {
        setReposError(msg);
      }
    } finally {
      setReposLoading(false);
    }
  }, [t]);

  useEffect(() => {
    void loadRepos();
  }, [loadRepos]);

  // ── On repo selection: detect fork + fetch branches ────────────────
  const onRepoSelect = useCallback(
    async (repoFullName: string) => {
      setSelectedRepo(repoFullName);
      setForkInfo(null);
      setBranches([]);
      setTargetBranch('');
      setSaveStatus('idle');

      if (!repoFullName) return;

      const [owner, repo] = repoFullName.split('/');
      if (!owner || !repo) return;

      setForkLoading(true);
      try {
        // Detect fork via composio_execute (curated tool)
        log('fetching repo metadata for %s', repoFullName);
        const res = await composioExecute('GITHUB_GET_A_REPOSITORY', { owner, repo });

        let branchOwner = owner;
        let branchRepo = repo;
        let detectedFork: ForkInfo | null = null;
        let defaultBranch = 'main';

        if (res.successful) {
          const repoData = res.data as {
            fork?: boolean;
            parent?: { full_name: string; owner: { login: string }; name: string };
            default_branch?: string;
          };

          if (repoData.fork && repoData.parent) {
            detectedFork = {
              isFork: true,
              upstreamOwner: repoData.parent.owner.login,
              upstreamRepo: repoData.parent.name,
              upstreamFullName: repoData.parent.full_name,
            };
            branchOwner = repoData.parent.owner.login;
            branchRepo = repoData.parent.name;
            log('detected fork → upstream: %s', repoData.parent.full_name);
          }
          defaultBranch = repoData.default_branch ?? 'main';
        } else {
          // If GITHUB_GET_A_REPOSITORY fails, fall back to repo metadata from the list
          log('GITHUB_GET_A_REPOSITORY failed, using list metadata. Error: %s', res.error);
          const repoFromList = repos.find(r => r.fullName === repoFullName);
          defaultBranch = repoFromList?.defaultBranch ?? 'main';
        }

        setForkInfo(detectedFork);

        // Fetch branches
        setBranchesLoading(true);
        log('fetching branches for %s/%s', branchOwner, branchRepo);
        const branchRes = await composioExecute('GITHUB_LIST_BRANCHES', {
          owner: branchOwner,
          repo: branchRepo,
          per_page: 100,
        });

        if (branchRes.successful) {
          // Composio wraps GitHub branch data as { data: { details: [...] } }
          const raw = branchRes.data;
          let branchList: GhBranch[] = [];
          if (Array.isArray(raw)) {
            branchList = raw as GhBranch[];
          } else if (raw && typeof raw === 'object') {
            const obj = raw as Record<string, unknown>;
            // Probe: details (Composio wrapper), data.details, branches, items, direct array under data
            const details = (obj as Record<string, unknown>).details;
            const dataObj = (obj as Record<string, unknown>).data as
              | Record<string, unknown>
              | undefined;
            const arr = details ?? dataObj?.details ?? obj.branches ?? obj.items ?? dataObj;
            if (Array.isArray(arr)) {
              branchList = arr as GhBranch[];
            }
          }
          log('fetched %d branches', branchList.length);

          if (branchList.length > 0) {
            setBranches(branchList);
            const hasDefault = branchList.some(b => b.name === defaultBranch);
            if (hasDefault) {
              setTargetBranch(defaultBranch);
            } else {
              setTargetBranch(branchList[0].name);
            }
          } else {
            // Successful but empty/unparseable — log raw data and use fallback
            log('branch response successful but no branches parsed. Raw data: %o', raw);
            const fallback = [...new Set([defaultBranch, 'main', 'master'])];
            setBranches(fallback.map(name => ({ name })));
            setTargetBranch(defaultBranch);
          }
        } else {
          // Branch listing failed — offer default branch as manual fallback
          log('GITHUB_LIST_BRANCHES failed: %s, using default branch fallback', branchRes.error);
          const fallback = [...new Set([defaultBranch, 'main', 'master'])];
          setBranches(fallback.map(name => ({ name })));
          setTargetBranch(defaultBranch);
        }
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        log('onRepoSelect error: %s', msg);
        setReposError(msg);
      } finally {
        setForkLoading(false);
        setBranchesLoading(false);
      }
    },
    [repos]
  );

  // ── Load run history ───────────────────────────────────────────────
  const loadRunHistory = useCallback(async () => {
    if (!existingJob) return;
    try {
      const res = await openhumanCronRuns(existingJob.id, 5);
      // RPC returns { result: { runs: CronRun[] }, logs: [...] }
      const raw = (res as { result?: { runs?: CoreCronRun[] } }).result;
      const runs = raw?.runs ?? [];
      setRunHistory(Array.isArray(runs) ? runs : []);
      log(
        'loaded %d run history entries for job %s',
        Array.isArray(runs) ? runs.length : 0,
        existingJob.id
      );
    } catch (err) {
      log('failed to load run history: %s', err);
    }
  }, [existingJob]);

  useEffect(() => {
    if (existingJob) {
      void loadRunHistory();
    }
  }, [existingJob, loadRunHistory]);

  // ── Save config ────────────────────────────────────────────────────
  const handleSave = useCallback(async () => {
    if (!selectedRepo || !targetBranch) return;

    const [owner] = selectedRepo.split('/');
    const upstreamName = forkInfo ? forkInfo.upstreamFullName : selectedRepo;

    const repoName = upstreamName.split('/')[1] ?? selectedRepo.split('/')[1] ?? '';
    const skillPrompt = [
      `You are running the dev-workflow skill. Follow these guidelines exactly.`,
      ``,
      `# Dev Workflow — Autonomous Issue Crusher`,
      ``,
      `Find a GitHub issue on \`${upstreamName}\`, implement a fix, and deliver a PR.`,
      ``,
      `## Repos`,
      `- **Upstream** = \`${upstreamName}\` — issues live here, PRs target \`${targetBranch}\`.`,
      `- **Fork** = \`${owner}/${repoName}\` — push the fix branch here.`,
      `- Commit through the GitHub API — no local git push.`,
      ``,
      `## Issue Selection (smart fallback)`,
      `1. **First**: Look for open issues assigned to \`${owner}\` on \`${upstreamName}\` with no linked PR.`,
      `2. **If none assigned**: Find unassigned open issues. Prefer issues labeled \`good first issue\`, \`bug\`, \`help wanted\`, or \`easy\`. Prefer issues with detailed descriptions (>500 chars). Skip issues that already have an open PR linked.`,
      `3. **Self-assign**: Once you pick an unassigned issue, assign it to \`${owner}\` using GITHUB_ADD_ASSIGNEES so no one else picks it up concurrently.`,
      `4. **If no suitable issues at all**: Exit cleanly — report "no suitable issues found".`,
      ``,
      `## Implementation Steps`,
      `1. Read the full issue body, comments, and labels.`,
      `2. Ensure fork \`${owner}/${repoName}\` exists (create if needed).`,
      `3. Clone \`${upstreamName}\` locally, branch \`dev-workflow/<issue>-<slug>\` off \`${targetBranch}\`.`,
      `4. Run \`codegraph_index\` on the repo.`,
      `5. Use \`codegraph_search\` to find relevant code. Fall back to grep/glob if coverage isn't full.`,
      `6. Implement the minimal correct fix. Re-read files and git diff — don't trust memory.`,
      `7. Run tests. Iterate until green.`,
      `8. Push via GitHub API (blob → tree → commit → update-ref). Do NOT git push.`,
      `9. Open cross-repo PR: \`${upstreamName}:${targetBranch}\` ← \`${owner}:<branch>\`. Body: Closes #N + summary + how you verified.`,
      ``,
      `## Rules`,
      `- One PR per run, then stop.`,
      `- Only fix the picked issue — no unrelated changes.`,
      `- codegraph is an accelerant, not a gate — fall back to grep if cold.`,
      `- If too large/risky (would touch >20 files or needs multi-system changes), comment on the issue explaining why and skip.`,
      `- Never force-push or push to upstream directly.`,
    ].join('\n');

    const cronParams: CronAddParams = {
      name: `dev-workflow-${selectedRepo.replace('/', '-')}`,
      schedule: { kind: 'cron', expr: schedule },
      job_type: 'agent',
      prompt: skillPrompt,
      session_target: 'isolated',
      delivery: { mode: 'proactive', best_effort: true },
    };

    log(
      'saving dev-workflow cron job: existingJob=%s, repo=%s',
      existingJob?.id ?? 'none',
      selectedRepo
    );

    try {
      if (existingJob) {
        // Update existing job
        await openhumanCronUpdate(existingJob.id, {
          name: cronParams.name,
          schedule: cronParams.schedule,
          prompt: cronParams.prompt,
        });
        log('updated cron job %s', existingJob.id);
      } else {
        // Create new job
        await openhumanCronAdd(cronParams);
        log('created new dev-workflow cron job for repo=%s', selectedRepo);
      }
      setSaveStatus('saved');
      void loadExistingJob(); // Refresh
      setTimeout(() => setSaveStatus('idle'), 3000);
    } catch (err) {
      log('save error: %s', err);
      setSaveStatus('error');
    }
  }, [selectedRepo, targetBranch, forkInfo, schedule, existingJob, loadExistingJob]);

  // ── Remove config ──────────────────────────────────────────────────
  const handleRemove = useCallback(async () => {
    if (!existingJob) return;
    log('removing dev-workflow cron job %s', existingJob.id);
    try {
      await openhumanCronRemove(existingJob.id);
      setExistingJob(null);
      setSelectedRepo('');
      setForkInfo(null);
      setBranches([]);
      setTargetBranch('');
      setSchedule(SCHEDULE_PRESETS[0].value);
      setSaveStatus('idle');
      setRunHistory([]);
      log('removed dev workflow cron job');
    } catch (err) {
      log('remove error: %s', err);
    }
  }, [existingJob]);

  // ── Toggle enable/disable ──────────────────────────────────────────
  const handleToggle = useCallback(async () => {
    if (!existingJob) return;
    const newEnabled = !existingJob.enabled;
    log('toggling cron job %s enabled=%s', existingJob.id, newEnabled);
    try {
      await openhumanCronUpdate(existingJob.id, { enabled: newEnabled });
      void loadExistingJob();
    } catch (err) {
      log('toggle error: %s', err);
    }
  }, [existingJob, loadExistingJob]);

  // ── Run Now ────────────────────────────────────────────────────────
  const handleRunNow = useCallback(async () => {
    if (!existingJob) return;
    setRunning(true);
    log('running cron job %s now', existingJob.id);
    try {
      await openhumanCronRun(existingJob.id);
      void loadExistingJob();
      void loadRunHistory();
    } catch (err) {
      log('run now error: %s', err);
    } finally {
      setRunning(false);
    }
  }, [existingJob, loadExistingJob, loadRunHistory]);

  // ── Render ─────────────────────────────────────────────────────────
  const canSave = selectedRepo && targetBranch && schedule;

  return (
    <SettingsPanel
      testId="dev-workflow-panel"
      description={t('settings.developerMenu.devWorkflow.desc')}>
      <div className="flex flex-col gap-5">
        {/* Description */}
        <p className="text-sm text-content-secondary dark:text-content-muted">
          {t('settings.developerMenu.devWorkflow.panelDesc')}
        </p>

        {/* Active config summary — shown at top regardless of repo loading */}
        {cronLoading && (
          <div className="text-xs text-content-muted">
            {t('settings.devWorkflow.loadingRepositories')}
          </div>
        )}
        {existingJob && (
          <SettingsSection>
            {/* Running indicator */}
            {running && (
              <div className="mx-4 mt-4 px-3 py-2 rounded-md bg-primary-50 dark:bg-primary-500/10 border border-primary-200 dark:border-primary-500/30 flex items-center gap-2">
                <span className="inline-block h-2 w-2 rounded-full bg-primary-500 animate-pulse" />
                <span className="text-xs font-medium text-primary-700 dark:text-primary-300">
                  {t('settings.devWorkflow.runningStatus')}
                </span>
              </div>
            )}
            <SettingsRow
              label={t('settings.devWorkflow.activeConfiguration')}
              htmlFor="dev-workflow-enabled"
              control={
                <div className="flex items-center gap-2">
                  <SettingsSwitch
                    id="dev-workflow-enabled"
                    checked={existingJob.enabled}
                    onCheckedChange={() => void handleToggle()}
                    aria-label={t('settings.devWorkflow.enabled')}
                  />
                  <span className="text-xs text-content-muted">
                    {existingJob.enabled
                      ? t('settings.devWorkflow.enabled')
                      : t('settings.devWorkflow.paused')}
                  </span>
                </div>
              }
            />
            <dl className="px-4 pb-3 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
              <dt className="text-content-muted">
                {t('settings.devWorkflow.activeConfigRepository')}
              </dt>
              <dd className="font-mono text-content">
                {existingJob.name?.replace(/^dev-workflow-/, '') ?? '—'}
              </dd>
              <dt className="text-content-muted">
                {t('settings.devWorkflow.activeConfigSchedule')}
              </dt>
              <dd className="text-content">
                {SCHEDULE_PRESETS.find(p => p.value === existingJob.expression)
                  ? t(SCHEDULE_PRESETS.find(p => p.value === existingJob.expression)!.labelKey)
                  : existingJob.expression}
              </dd>
              <dt className="text-content-muted">{t('settings.devWorkflow.nextRun')}</dt>
              <dd className="text-content">
                {existingJob.next_run ? new Date(existingJob.next_run).toLocaleString() : '—'}
              </dd>
              {existingJob.last_run && (
                <>
                  <dt className="text-content-muted">{t('settings.devWorkflow.lastRun')}</dt>
                  <dd className="text-content">
                    {new Date(existingJob.last_run).toLocaleString()}
                    {existingJob.last_status && (
                      <span
                        className={`ml-2 px-1.5 py-0.5 rounded text-[10px] font-medium ${
                          existingJob.last_status === 'ok'
                            ? 'bg-sage-100 dark:bg-sage-500/20 text-sage-700 dark:text-sage-300'
                            : 'bg-coral-100 dark:bg-coral-500/20 text-coral-700 dark:text-coral-300'
                        }`}>
                        {existingJob.last_status}
                      </span>
                    )}
                  </dd>
                </>
              )}
            </dl>

            <div className="px-4 pb-4 flex items-center gap-2">
              <Button
                type="button"
                variant="secondary"
                size="xs"
                onClick={() => void handleRunNow()}
                disabled={running}>
                {running ? t('settings.devWorkflow.running') : t('settings.devWorkflow.runNow')}
              </Button>
              <Button
                type="button"
                variant="secondary"
                tone="danger"
                size="xs"
                onClick={() => void handleRemove()}>
                {t('settings.devWorkflow.remove')}
              </Button>
            </div>

            {existingJob.last_output && (
              <div className="px-4 pb-4">
                <div className="text-xs font-medium text-content-muted mb-1">
                  {t('settings.devWorkflow.lastOutput')}
                </div>
                <pre className="px-3 py-2 rounded-md bg-surface-subtle border border-line dark:border-line-strong text-[11px] text-content-secondary font-mono whitespace-pre-wrap break-words max-h-48 overflow-y-auto">
                  {existingJob.last_output}
                </pre>
              </div>
            )}

            {runHistory.length > 0 && (
              <div className="px-4 pb-4">
                <button
                  type="button"
                  onClick={() => setHistoryExpanded(!historyExpanded)}
                  className="text-xs text-content-muted hover:text-content-secondary dark:hover:text-neutral-200 transition-colors">
                  {historyExpanded ? '▾' : '▸'} {t('settings.devWorkflow.recentRuns')} (
                  {runHistory.length})
                </button>
                {historyExpanded && (
                  <div className="mt-1.5 space-y-1">
                    {runHistory.map(run => (
                      <div key={run.id} className="rounded bg-surface">
                        <button
                          type="button"
                          onClick={() => setExpandedRunId(expandedRunId === run.id ? null : run.id)}
                          className="w-full flex items-center justify-between px-2 py-1.5 text-xs hover:bg-surface-muted dark:hover:bg-neutral-750 rounded transition-colors">
                          <div className="flex items-center gap-2">
                            <span className="text-content-faint">
                              {expandedRunId === run.id ? '▾' : '▸'}
                            </span>
                            <span className="text-content-secondary dark:text-content-muted">
                              {new Date(run.started_at).toLocaleString()}
                            </span>
                          </div>
                          <div className="flex items-center gap-2">
                            {run.duration_ms != null && (
                              <span className="text-content-muted dark:text-content-faint">
                                {(run.duration_ms / 1000).toFixed(1)}s
                              </span>
                            )}
                            <span
                              className={`px-1.5 py-0.5 rounded text-[10px] font-medium ${
                                run.status === 'ok'
                                  ? 'bg-sage-100 dark:bg-sage-500/20 text-sage-700 dark:text-sage-300'
                                  : 'bg-coral-100 dark:bg-coral-500/20 text-coral-700 dark:text-coral-300'
                              }`}>
                              {run.status}
                            </span>
                          </div>
                        </button>
                        {expandedRunId === run.id && run.output && (
                          <pre className="mx-2 mb-2 px-3 py-2 rounded-md bg-surface-subtle dark:bg-surface border border-line dark:border-line-strong text-[11px] text-content-secondary font-mono whitespace-pre-wrap break-words max-h-64 overflow-y-auto">
                            {run.output}
                          </pre>
                        )}
                        {expandedRunId === run.id && !run.output && (
                          <div className="mx-2 mb-2 px-3 py-2 text-[11px] text-content-faint italic">
                            {t('settings.devWorkflow.noOutput')}
                          </div>
                        )}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}
          </SettingsSection>
        )}

        {/* Setup form — only shown when no active config exists */}
        {!existingJob && (
          <>
            <SettingsSection title={t('settings.devWorkflow.githubRepository')}>
              {reposError && (
                <div className="mx-4 mt-3 px-3 py-2 rounded-md bg-coral-50 dark:bg-coral-500/10 border border-coral-200 dark:border-coral-500/30 text-xs text-coral-700 dark:text-coral-300">
                  {reposError}
                </div>
              )}
              <SettingsRow
                stacked
                control={
                  <SettingsSelect
                    value={selectedRepo}
                    onChange={e => void onRepoSelect(e.target.value)}
                    disabled={reposLoading}
                    className="w-full">
                    <option value="">
                      {reposLoading
                        ? t('settings.devWorkflow.loadingRepositories')
                        : t('settings.devWorkflow.selectRepository')}
                    </option>
                    {repos.map(r => (
                      <option key={r.fullName} value={r.fullName}>
                        {r.fullName} {r.private ? t('settings.devWorkflow.privateTag') : ''}
                      </option>
                    ))}
                  </SettingsSelect>
                }
              />
            </SettingsSection>

            {/* Fork info */}
            {forkLoading && (
              <div className="text-xs text-content-muted">
                {t('settings.devWorkflow.detectingForkInfo')}
              </div>
            )}
            {forkInfo && (
              <div className="px-3 py-2 rounded-md bg-primary-50 dark:bg-primary-500/10 border border-primary-200 dark:border-primary-500/30">
                <div className="text-xs font-medium text-primary-800 dark:text-primary-300">
                  {t('settings.devWorkflow.forkDetected')}
                </div>
                <div className="text-xs text-primary-700 dark:text-primary-200 mt-0.5">
                  {t('settings.devWorkflow.upstream')}{' '}
                  <span className="font-mono">{forkInfo.upstreamFullName}</span>
                </div>
                <div className="text-xs text-primary-600 dark:text-primary-300 mt-0.5">
                  {t('settings.devWorkflow.forkPrNote')}
                </div>
              </div>
            )}
            {selectedRepo && !forkLoading && !forkInfo && (
              <div className="px-3 py-2 rounded-md bg-surface-muted border border-line dark:border-line-strong">
                <div className="text-xs text-content-secondary dark:text-content-muted">
                  {t('settings.devWorkflow.notForkNote')}
                </div>
              </div>
            )}

            {/* Branch selector */}
            {branches.length > 0 && (
              <SettingsSection title={t('settings.devWorkflow.targetBranch')}>
                <SettingsRow
                  stacked
                  description={`${t('settings.devWorkflow.targetBranchNote')}${forkInfo ? ` on ${forkInfo.upstreamFullName}` : ''}.`}
                  control={
                    <SettingsSelect
                      value={targetBranch}
                      onChange={e => {
                        setTargetBranch(e.target.value);
                        setSaveStatus('idle');
                      }}
                      disabled={branchesLoading}
                      className="w-full">
                      {branches.map(b => (
                        <option key={b.name} value={b.name}>
                          {b.name}
                        </option>
                      ))}
                    </SettingsSelect>
                  }
                />
              </SettingsSection>
            )}
            {branchesLoading && (
              <div className="text-xs text-content-muted">
                {t('settings.devWorkflow.loadingBranches')}
              </div>
            )}

            {/* Schedule */}
            {selectedRepo && (
              <SettingsSection title={t('settings.devWorkflow.runFrequency')}>
                <SettingsRow
                  stacked
                  description={t('settings.devWorkflow.runFrequencyNote')}
                  control={
                    <SettingsSelect
                      value={schedule}
                      onChange={e => {
                        setSchedule(e.target.value);
                        setSaveStatus('idle');
                      }}
                      className="w-full">
                      {SCHEDULE_PRESETS.map(p => (
                        <option key={p.value} value={p.value}>
                          {t(p.labelKey)}
                        </option>
                      ))}
                    </SettingsSelect>
                  }
                />
              </SettingsSection>
            )}

            {/* Actions */}
            {selectedRepo && (
              <div className="flex items-center gap-3 pb-4">
                <Button
                  type="button"
                  variant="primary"
                  size="sm"
                  onClick={() => void handleSave()}
                  disabled={!canSave}>
                  {t('settings.devWorkflow.saveConfiguration')}
                </Button>
                <SettingsStatusLine
                  saving={false}
                  savedNote={saveStatus === 'saved' ? t('settings.devWorkflow.saved') : null}
                  error={saveStatus === 'error' ? t('settings.devWorkflow.cronSaveError') : null}
                  savingLabel=""
                />
              </div>
            )}
          </>
        )}
      </div>
    </SettingsPanel>
  );
};

export default DevWorkflowPanel;
