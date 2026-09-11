/**
 * UninstallSkillConfirmDialog
 * ---------------------------
 *
 * Small centered confirm modal for destructive uninstall of a user-scope
 * SKILL.md skill. Wraps `skillsApi.uninstallWorkflow` which calls
 * `openhuman.skill_registry_uninstall` on the Rust side — that RPC only accepts
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
import debug from 'debug';
import { useCallback, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { trackEvent } from '../../services/analytics';
import {
  type UninstallWorkflowResult,
  skillsApi,
  type WorkflowSummary,
} from '../../services/api/skillsApi';
import { ModalShell } from '../ui';
import Button from '../ui/Button';

const log = debug('skills:uninstall-dialog');

interface Props {
  skill: WorkflowSummary;
  onClose: () => void;
  /**
   * Fires when the backend reports the uninstall succeeded. Parent is
   * responsible for refetching the skills list and closing any detail
   * panels that were showing this skill.
   */
  onUninstalled: (result: UninstallWorkflowResult) => void;
}

export default function UninstallSkillConfirmDialog({ skill, onClose, onUninstalled }: Props) {
  const { t } = useT();
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleConfirm = useCallback(async () => {
    log('confirm: id=%s name=%s', skill.id, skill.name);
    setSubmitting(true);
    setError(null);
    try {
      // `skill.id` is the on-disk slug (directory under ~/.openhuman/skills/).
      // `skill.name` is the frontmatter display name and may diverge from the
      // slug — the backend resolves by slug, so pass `id`.
      const result = await skillsApi.uninstallWorkflow(skill.id);
      log('confirm: done removedPath=%s', result.removedPath);
      trackEvent('skill_uninstall', { skill_id: skill.id });
      onUninstalled(result);
      onClose();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      log('confirm: error=%s', msg);
      setError(msg);
      setSubmitting(false);
    }
  }, [skill.id, skill.name, onUninstalled, onClose, t]);

  return (
    <ModalShell
      onClose={() => {
        if (!submitting) onClose();
      }}
      title={
        <>
          {t('common.delete')} {skill.name}?
        </>
      }
      titleId="uninstall-skill-title"
      maxWidthClassName="max-w-[420px]"
      contentClassName="px-5 pb-1 pt-4"
      closePolicy={submitting ? { escape: false, backdrop: false, button: false } : undefined}
      footer={
        <div className="flex items-center justify-end gap-2">
          <Button variant="secondary" size="sm" disabled={submitting} onClick={onClose}>
            {t('common.cancel')}
          </Button>
          <Button
            variant="secondary"
            tone="danger"
            size="sm"
            disabled={submitting}
            onClick={handleConfirm}
            data-testid="uninstall-skill-confirm">
            {submitting ? t('team.deleting') : t('common.delete')}
          </Button>
        </div>
      }>
      <p className="text-sm text-content-secondary">{t('skills.uninstall.description')}</p>
      {skill.location && (
        <p className="mt-3 break-all rounded-lg bg-surface-muted px-3 py-2 font-mono text-[11px] text-content-secondary">
          {skill.location.replace(/\/(WORKFLOW|SKILL)\.md$/i, '')}
        </p>
      )}
      {error && (
        <div className="mt-3 rounded-lg border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700">
          <div className="font-medium">{t('workflows.deleteError')}</div>
          <div className="mt-1 wrap-break-word font-mono text-[11px] text-coral-700/90">{error}</div>
        </div>
      )}
    </ModalShell>
  );
}
