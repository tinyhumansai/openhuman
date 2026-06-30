// SmartIssuePicker — dev-workflow's repo / fork / upstream / branch
// auto-detection lifted out of DevWorkflowPanel into a reusable
// subcomponent. WorkflowRunnerBody conditionally mounts it when the
// selected skill is `dev-workflow` to give that one skill the same
// frictionless setup it had in its bespoke Settings panel:
//
//   - One dropdown shows the user's GitHub-connected repos (via
//     Composio GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER).
//   - Picking a repo runs GITHUB_GET_A_REPOSITORY to detect whether
//     it's a fork and, if so, resolves the upstream's owner/name.
//   - Branches are then listed (from the upstream side when forked) so
//     the target_branch field can be a real dropdown rather than a
//     freeform text field.
//
// All four dev-workflow inputs (`repo`, `upstream`, `target_branch`,
// `fork_owner`) are populated through the single `onPatchInputs`
// callback so the parent's form state is the source of truth and Run /
// Save behaviour is untouched.
//
// TODO(picker-schema): today this subcomponent is wired in based on
// the skill id being literally `dev-workflow`. The cleaner long-term
// path is to extend `skill.toml`'s `[[inputs]]` with an optional
// `picker = "github-issue"` discriminator and route here from that;
// see docs/skills-runner-unification.md open question 1.
import createDebug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { execute as composioExecute, listConnections } from '../../lib/composio/composioApi';
import { useT } from '../../lib/i18n/I18nContext';

const log = createDebug('app:skills:SmartIssuePicker');

interface ComposioGhRepo {
  owner: string;
  repo: string;
  fullName: string;
  private?: boolean;
  defaultBranch?: string;
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

export interface SmartIssuePickerProps {
  /** Current resolved input values (the four dev-workflow fields). */
  values: { repo?: string; upstream?: string; target_branch?: string; fork_owner?: string };
  /** Patch the parent's form-values map with the picker's resolutions. */
  onPatchInputs: (
    patch: Partial<{ repo: string; upstream: string; target_branch: string; fork_owner: string }>
  ) => void;
}

const SmartIssuePicker = ({ values, onPatchInputs }: SmartIssuePickerProps) => {
  const { t } = useT();

  const [repos, setRepos] = useState<ComposioGhRepo[]>([]);
  const [reposLoading, setReposLoading] = useState(false);
  const [reposError, setReposError] = useState<string | null>(null);

  const [forkInfo, setForkInfo] = useState<ForkInfo | null>(null);
  const [forkLoading, setForkLoading] = useState(false);

  const [branches, setBranches] = useState<GhBranch[]>([]);
  const [branchesLoading, setBranchesLoading] = useState(false);

  // Monotonic counter guarding against race conditions when the user
  // changes repos faster than the async fork/branch lookups resolve:
  // each onRepoSelect captures its own id and bails out of any state
  // update once a newer selection has superseded it.
  const selectionSeqRef = useRef(0);

  // ── Load repos via Composio ─────────────────────────────────────────
  const loadRepos = useCallback(async () => {
    setReposLoading(true);
    setReposError(null);
    try {
      const connections = await listConnections();
      const ghConn = connections.connections?.find(
        c =>
          c.toolkit.toLowerCase().includes('github') &&
          (c.status === 'ACTIVE' || c.status === 'CONNECTED')
      );
      if (!ghConn) throw new Error('NOT_CONNECTED');

      const res = await composioExecute('GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER', {});
      if (!res.successful) throw new Error(res.error ?? 'Failed to fetch repositories');

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
        }));
      }

      setRepos(repoList);
      if (repoList.length === 0) {
        setReposError(t('settings.devWorkflow.errorNoRepositories'));
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      log('loadRepos error: %s', msg);
      if (msg === 'NOT_CONNECTED') {
        setReposError(t('settings.devWorkflow.errorNotConnected'));
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

  // ── Fork detect + branch list on repo select ────────────────────────
  const onRepoSelect = useCallback(
    async (repoFullName: string) => {
      // Claim this selection's id; any later selection bumps the ref and
      // makes the work below abandon its now-stale state updates.
      const localId = ++selectionSeqRef.current;

      // Reset downstream resolutions and bubble the new repo up.
      onPatchInputs({
        repo: repoFullName,
        upstream: repoFullName,
        target_branch: '',
        fork_owner: repoFullName.split('/')[0] ?? '',
      });
      setForkInfo(null);
      setBranches([]);

      if (!repoFullName) return;
      const [owner, repo] = repoFullName.split('/');
      if (!owner || !repo) return;

      setForkLoading(true);
      try {
        const res = await composioExecute('GITHUB_GET_A_REPOSITORY', { owner, repo });
        if (localId !== selectionSeqRef.current) return;
        let branchOwner = owner;
        let branchRepo = repo;
        let detectedFork: ForkInfo | null = null;
        let defaultBranch = 'main';

        if (res.successful) {
          const data = res.data as {
            fork?: boolean;
            parent?: { full_name: string; owner: { login: string }; name: string };
            default_branch?: string;
          };
          if (data.fork && data.parent) {
            detectedFork = {
              isFork: true,
              upstreamOwner: data.parent.owner.login,
              upstreamRepo: data.parent.name,
              upstreamFullName: data.parent.full_name,
            };
            branchOwner = data.parent.owner.login;
            branchRepo = data.parent.name;
          }
          defaultBranch = data.default_branch ?? 'main';
        } else {
          const fromList = repos.find(r => r.fullName === repoFullName);
          defaultBranch = fromList?.defaultBranch ?? 'main';
        }

        setForkInfo(detectedFork);
        onPatchInputs({
          upstream: detectedFork ? detectedFork.upstreamFullName : repoFullName,
          fork_owner: owner,
        });

        setBranchesLoading(true);
        const branchRes = await composioExecute('GITHUB_LIST_BRANCHES', {
          owner: branchOwner,
          repo: branchRepo,
          per_page: 100,
        });
        if (localId !== selectionSeqRef.current) return;
        if (branchRes.successful) {
          const raw = branchRes.data;
          let list: GhBranch[] = [];
          if (Array.isArray(raw)) {
            list = raw as GhBranch[];
          } else if (raw && typeof raw === 'object') {
            const obj = raw as Record<string, unknown>;
            const dataObj = obj.data as Record<string, unknown> | undefined;
            const arr =
              (obj.details as unknown) ?? dataObj?.details ?? obj.branches ?? obj.items ?? dataObj;
            if (Array.isArray(arr)) list = arr as GhBranch[];
          }
          if (list.length > 0) {
            setBranches(list);
            const hasDefault = list.some(b => b.name === defaultBranch);
            onPatchInputs({ target_branch: hasDefault ? defaultBranch : list[0].name });
          } else {
            const fallback = [...new Set([defaultBranch, 'main', 'master'])];
            setBranches(fallback.map(name => ({ name })));
            onPatchInputs({ target_branch: defaultBranch });
          }
        } else {
          const fallback = [...new Set([defaultBranch, 'main', 'master'])];
          setBranches(fallback.map(name => ({ name })));
          onPatchInputs({ target_branch: defaultBranch });
        }
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        log('onRepoSelect error: %s', msg);
        if (localId !== selectionSeqRef.current) return;
        setReposError(msg);
      } finally {
        if (localId === selectionSeqRef.current) {
          setForkLoading(false);
          setBranchesLoading(false);
        }
      }
    },
    [repos, onPatchInputs]
  );

  // ── Render ──────────────────────────────────────────────────────────
  return (
    <div data-testid="smart-issue-picker" className="space-y-3">
      <div>
        <label
          htmlFor="smart-issue-picker-repo"
          className="block text-sm font-medium text-content-secondary dark:text-stone-300 mb-1">
          {t('settings.devWorkflow.githubRepository')}
        </label>
        {reposError && (
          <div className="mb-2 px-3 py-2 rounded-md bg-coral-50 dark:bg-coral-500/10 border border-coral-200 dark:border-coral-500/30 text-xs text-coral-700 dark:text-coral-300">
            {reposError}
          </div>
        )}
        <select
          id="smart-issue-picker-repo"
          value={values.repo ?? ''}
          onChange={e => void onRepoSelect(e.target.value)}
          disabled={reposLoading}
          className="w-full rounded border border-line-strong dark:border-stone-600 bg-surface px-3 py-2 text-sm text-content dark:text-stone-100">
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
        </select>
      </div>

      {forkLoading && (
        <div className="text-xs text-content-muted dark:text-stone-400">
          {t('settings.devWorkflow.detectingForkInfo')}
        </div>
      )}
      {forkInfo && (
        <div
          data-testid="smart-issue-picker-fork-banner"
          className="px-3 py-2 rounded-md bg-primary-50 dark:bg-primary-500/10 border border-primary-200 dark:border-primary-500/30">
          <div className="text-xs font-medium text-primary-800 dark:text-primary-300">
            {t('settings.devWorkflow.forkDetected')}
          </div>
          <div className="text-xs text-primary-700 dark:text-primary-200 mt-0.5">
            {t('settings.devWorkflow.upstream')}{' '}
            <span className="font-mono">{forkInfo.upstreamFullName}</span>
          </div>
        </div>
      )}

      {branches.length > 0 && (
        <div>
          <label
            htmlFor="smart-issue-picker-branch"
            className="block text-sm font-medium text-content-secondary dark:text-stone-300 mb-1">
            {t('settings.devWorkflow.targetBranch')}
          </label>
          <select
            id="smart-issue-picker-branch"
            value={values.target_branch ?? ''}
            onChange={e => onPatchInputs({ target_branch: e.target.value })}
            disabled={branchesLoading}
            className="w-full rounded border border-line-strong dark:border-stone-600 bg-surface px-3 py-2 text-sm text-content dark:text-stone-100">
            {branches.map(b => (
              <option key={b.name} value={b.name}>
                {b.name}
              </option>
            ))}
          </select>
        </div>
      )}
    </div>
  );
};

export default SmartIssuePicker;
