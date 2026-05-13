/**
 * UninstallSkillConfirmDialog
 * ---------------------------
 *
 * Small centered confirm modal for destructive uninstall of a user-scope
 * SKILL.md skill. Wraps `skillsApi.uninstallSkill` which calls
 * `openhuman.skills_uninstall` on the Rust side — that RPC only accepts
 * user-scope installs (`~/.openhuman/skills/<name>/`) and refuses project
 * and legacy scopes. The card that opens this dialog is responsible for
 * not surfacing the Uninstall action for non-user-scope entries.
 *
 * UI contract:
 *   - Shows skill name, resolved on-disk path (when known), and a plain
 *     warning line.
 *   - "Cancel" dismisses. "Uninstall" fires the RPC.
 *   - While the RPC is in flight, both buttons disable and the modal is
 *     non-dismissable (Esc / backdrop ignored) so the caller sees the
 *     outcome.
 *   - On success, the parent's `onUninstalled(result)` callback runs and
 *     the dialog closes. On failure, the raw backend error is surfaced
 *     inline; the dialog stays open so the user can retry or cancel.
 *
 * Design mirrors `InstallSkillDialog` — see
 * `.claude/rules/15-settings-modal-system.md`.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import debug from 'debug';

import {
  skillsApi,
  type SkillSummary,
  type UninstallSkillResult,
} from '../../services/api/skillsApi';
import { trackEvent } from '../../services/analytics';

const log = debug('skills:uninstall-dialog');

interface Props {
  skill: SkillSummary;
  onClose: () => void;
  /**
   * Fires when the backend reports the uninstall succeeded. Parent is
   * responsible for refetching the skills list and closing any detail
   * panels that were showing this skill.
   */
  onUninstalled: (result: UninstallSkillResult) => void;
}

export default function UninstallSkillConfirmDialog({ skill, onClose, onUninstalled }: Props) {
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const cancelBtnRef = useRef<HTMLButtonElement | null>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    previousFocusRef.current = document.activeElement as HTMLElement | null;
    cancelBtnRef.current?.focus();
    return () => {
      previousFocusRef.current?.focus();
    };
  }, []);

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !submitting) {
        e.preventDefault();
        onClose();
      }
    };
    document.addEventListener('keydown', handleKey);
    return () => document.removeEventListener('keydown', handleKey);
  }, [onClose, submitting]);

  const handleConfirm = useCallback(async () => {
    log('confirm: id=%s name=%s', skill.id, skill.name);
    setSubmitting(true);
    setError(null);
    try {
      // `skill.id` is the on-disk slug (directory under ~/.openhuman/skills/).
      // `skill.name` is the frontmatter display name and may diverge from the
      // slug — the backend resolves by slug, so pass `id`.
      const result = await skillsApi.uninstallSkill(skill.id);
      log('confirm: done removedPath=%s', result.removedPath);
      trackEvent('skill_uninstall', { skill_id: skill.id });
      onUninstalled(result);
      onClose();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      log('confirm: error=%s', msg);
      setError(`Couldn't uninstall skill: ${msg}`);
      setSubmitting(false);
    }
  }, [skill.id, skill.name, onUninstalled, onClose]);

  return createPortal(
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="uninstall-skill-title"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
      onMouseDown={e => {
        if (e.target === e.currentTarget && !submitting) onClose();
      }}>
      <div className="w-[420px] max-w-[90vw] rounded-2xl bg-white p-5 shadow-2xl">
        <h2 id="uninstall-skill-title" className="text-base font-semibold text-stone-900">
          Uninstall {skill.name}?
        </h2>
        <p className="mt-2 text-sm text-stone-600">
          This permanently deletes the skill directory and all its bundled resources. The agent
          will stop seeing it at the next turn.
        </p>
        {skill.location && (
          <p className="mt-3 break-all rounded-lg bg-stone-50 px-3 py-2 font-mono text-[11px] text-stone-600">
            {skill.location.replace(/\/SKILL\.md$/i, '')}
          </p>
        )}
        {error && (
          <div className="mt-3 rounded-lg border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700">
            <div className="font-medium">Could not uninstall</div>
            <div className="mt-1 break-words font-mono text-[11px] text-coral-700/90">{error}</div>
          </div>
        )}
        <div className="mt-5 flex items-center justify-end gap-2">
          <button
            ref={cancelBtnRef}
            type="button"
            disabled={submitting}
            onClick={onClose}
            className="rounded-lg border border-stone-200 bg-white px-3 py-1.5 text-xs font-medium text-stone-700 hover:bg-stone-50 disabled:cursor-not-allowed disabled:opacity-50">
            Cancel
          </button>
          <button
            type="button"
            disabled={submitting}
            onClick={handleConfirm}
            data-testid="uninstall-skill-confirm"
            className="rounded-lg border border-coral-300 bg-coral-50 px-3 py-1.5 text-xs font-medium text-coral-700 hover:bg-coral-100 disabled:cursor-not-allowed disabled:opacity-50">
            {submitting ? 'Uninstalling…' : 'Uninstall'}
          </button>
        </div>
      </div>
    </div>,
    document.body
  );
}
