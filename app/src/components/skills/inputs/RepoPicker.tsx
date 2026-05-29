// Reusable GitHub repo picker — autocomplete dropdown sourced from the
// user's Composio-connected GitHub account via
// `composio_execute(GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER)`.
//
// Used by SkillsRunnerBody for any skill input whose name matches the
// repo-shaped conventions (`repo`, `repository`, `upstream`, `fork`,
// `fork_owner`). Replaces the plain text input with this picker so users
// don't have to type `owner/name` manually for skills like
// github-issue-crusher and dev-workflow.
//
// Logic mirrors DevWorkflowPanel's existing repo-loading flow (same
// Composio RPCs, same wire-shape parsing) so the picker behaves
// identically to the Settings → Dev Workflow panel. The original panel
// stays in place with its own inline implementation; this is a parallel
// component for the generic Skills Runner surface.

import createDebug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { execute as composioExecute, listConnections } from '../../../lib/composio/composioApi';
import { useT } from '../../../lib/i18n/I18nContext';

const log = createDebug('app:skills:RepoPicker');

/** Shape returned by `openhuman.composio_list_github_repos`. */
export interface ComposioGhRepo {
  owner: string;
  repo: string;
  fullName: string;
  private?: boolean;
  defaultBranch?: string;
  htmlUrl?: string;
}

export interface RepoPickerProps {
  /** Currently-selected `owner/name` (or empty). */
  value: string;
  /** Fires with the picked `owner/name`. */
  onChange: (next: string) => void;
  /** Optional `id` for `<label htmlFor>` to bind correctly. */
  id?: string;
  /** Optional `placeholder` text override. */
  placeholder?: string;
  /** Disable the picker entirely. */
  disabled?: boolean;
}

const RepoPicker = ({ value, onChange, id, placeholder, disabled }: RepoPickerProps) => {
  const { t } = useT();
  const [repos, setRepos] = useState<ComposioGhRepo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // ── Fetch repos via Composio (mirrors DevWorkflowPanel) ────────────
  const loadRepos = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      // Step 1: Is GitHub connected via Composio?
      const conns = await listConnections();
      const ghConn = conns.connections?.find(
        (c) =>
          c.toolkit.toLowerCase().includes('github') &&
          (c.status === 'ACTIVE' || c.status === 'CONNECTED')
      );
      if (!ghConn) throw new Error('NOT_CONNECTED');

      // Step 2: Fetch repos.
      const res = await composioExecute(
        'GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER',
        {}
      );
      if (!res.successful) throw new Error(res.error ?? 'Failed to fetch repositories');

      // Step 3: Parse — GitHub API returns an array of repo objects;
      // Composio sometimes wraps it under `.repositories`.
      const raw = res.data;
      const items = Array.isArray(raw)
        ? raw
        : ((raw as Record<string, unknown>)?.repositories ?? []);
      const list: ComposioGhRepo[] = Array.isArray(items)
        ? (items as Record<string, unknown>[]).map((r) => ({
            owner: String((r.owner as Record<string, unknown>)?.login ?? r.owner ?? ''),
            repo: String(r.name ?? ''),
            fullName: String(
              r.full_name ?? `${(r.owner as Record<string, unknown>)?.login ?? r.owner}/${r.name}`
            ),
            private: r.private as boolean | undefined,
            defaultBranch: r.default_branch as string | undefined,
            htmlUrl: r.html_url as string | undefined,
          }))
        : [];
      log('loaded %d repos', list.length);
      setRepos(list);
      if (list.length === 0) {
        setError(t('settings.skillsRunner.repoPicker.empty'));
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      log('loadRepos error: %s', msg);
      if (msg === 'NOT_CONNECTED') {
        setError(t('settings.skillsRunner.repoPicker.notConnected'));
      } else {
        setError(msg);
      }
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    void loadRepos();
  }, [loadRepos]);

  // Common <select> classes — match the plain inputs in SkillsRunnerBody
  // so the picker visually blends with the surrounding form.
  const selectClass =
    'w-full rounded border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-800 px-3 py-2 text-sm text-stone-900 dark:text-stone-100';

  return (
    <div>
      <select
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={disabled || loading || error !== null}
        className={selectClass}
      >
        <option value="">
          {loading
            ? t('settings.skillsRunner.repoPicker.loading')
            : (placeholder ?? t('settings.skillsRunner.repoPicker.select'))}
        </option>
        {repos.map((r) => (
          <option key={r.fullName} value={r.fullName}>
            {r.fullName}
            {r.private ? ` ${t('settings.skillsRunner.repoPicker.privateTag')}` : ''}
          </option>
        ))}
      </select>
      {error && (
        <p className="text-xs text-red-600 dark:text-red-400 mt-1">{error}</p>
      )}
    </div>
  );
};

export default RepoPicker;
