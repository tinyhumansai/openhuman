// Reusable GitHub branch picker — dropdown sourced from
// `composio_execute(GITHUB_LIST_BRANCHES)` for the linked repo input.
//
// Used by SkillsRunnerBody for any skill input whose name matches the
// branch-shaped conventions (`branch`, `target_branch`, `base_branch`,
// `pr_base`, `head_branch`). Depends on a sibling `repo`-shaped input
// for which repo to list branches for; if that sibling is empty, the
// picker renders a disabled dropdown with a "select a repo first" hint.
//
// Refetches whenever `repo` changes. Like RepoPicker, this is a
// parallel component to the inline impl in DevWorkflowPanel — the
// original panel stays untouched.

import createDebug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { execute as composioExecute } from '../../../lib/composio/composioApi';
import { useT } from '../../../lib/i18n/I18nContext';

const log = createDebug('app:skills:BranchPicker');

interface GhBranch {
  name: string;
}

export interface BranchPickerProps {
  /** Selected branch name (or empty). */
  value: string;
  /** Fires with the picked branch name. */
  onChange: (next: string) => void;
  /**
   * `owner/repo` of the repo to list branches for. When empty, the
   * picker renders disabled with a "select a repo first" hint.
   */
  repo: string;
  id?: string;
  placeholder?: string;
  disabled?: boolean;
}

const BranchPicker = ({ value, onChange, repo, id, placeholder, disabled }: BranchPickerProps) => {
  const { t } = useT();
  const [branches, setBranches] = useState<GhBranch[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Monotonic request token so stale GITHUB_LIST_BRANCHES responses
  // (a slower fetch for a previous repo) can't overwrite state for the
  // current repo. Each call captures the seq it started with and bails
  // before every post-await state write if a newer call has begun.
  const requestSeqRef = useRef(0);

  const loadBranches = useCallback(async () => {
    if (!repo || !repo.includes('/')) {
      setBranches([]);
      setError(null);
      return;
    }
    const [owner, repoName] = repo.split('/');
    if (!owner || !repoName) {
      setBranches([]);
      return;
    }
    const seq = ++requestSeqRef.current;
    setLoading(true);
    setError(null);
    try {
      const res = await composioExecute('GITHUB_LIST_BRANCHES', {
        owner,
        repo: repoName,
        per_page: 100,
      });
      if (seq !== requestSeqRef.current) return;
      if (!res.successful) throw new Error(res.error ?? 'Failed to list branches');
      // Composio wraps GitHub branch data in a few different shapes
      // (details, data.details, branches, items, direct array under data) —
      // probe the same way DevWorkflowPanel does.
      const raw = res.data;
      let list: GhBranch[] = [];
      if (Array.isArray(raw)) {
        list = raw as GhBranch[];
      } else if (raw && typeof raw === 'object') {
        const obj = raw as Record<string, unknown>;
        const dataObj = obj.data as Record<string, unknown> | undefined;
        const arr =
          (obj.details as unknown[] | undefined) ??
          (dataObj?.details as unknown[] | undefined) ??
          (obj.branches as unknown[] | undefined) ??
          (obj.items as unknown[] | undefined) ??
          (dataObj as unknown[] | undefined);
        if (Array.isArray(arr)) {
          list = arr as GhBranch[];
        }
      }
      log('loaded %d branches for %s', list.length, repo);
      setBranches(list);
      if (list.length === 0) {
        // Fall back so the user can still pick something sensible.
        setBranches([{ name: 'main' }, { name: 'master' }]);
      }
    } catch (err: unknown) {
      if (seq !== requestSeqRef.current) return;
      const msg = err instanceof Error ? err.message : String(err);
      log('loadBranches error: %s', msg);
      setError(msg);
      // Even on error, give the user the standard defaults.
      setBranches([{ name: 'main' }, { name: 'master' }]);
    } finally {
      if (seq === requestSeqRef.current) setLoading(false);
    }
  }, [repo]);

  useEffect(() => {
    void loadBranches();
  }, [loadBranches]);

  const selectClass =
    'w-full rounded border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-800 px-3 py-2 text-sm text-stone-900 dark:text-stone-100';

  return (
    <div>
      <select
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={disabled || loading || !repo}
        className={selectClass}
      >
        <option value="">
          {!repo
            ? t('settings.skillsRunner.branchPicker.needRepo')
            : loading
              ? t('settings.skillsRunner.branchPicker.loading')
              : (placeholder ?? t('settings.skillsRunner.branchPicker.select'))}
        </option>
        {branches.map((b) => (
          <option key={b.name} value={b.name}>
            {b.name}
          </option>
        ))}
      </select>
      {error && (
        <p className="text-xs text-red-600 dark:text-red-400 mt-1">{error}</p>
      )}
    </div>
  );
};

export default BranchPicker;
