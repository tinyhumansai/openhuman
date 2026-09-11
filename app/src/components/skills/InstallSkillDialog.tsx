/**
 * InstallSkillDialog
 * ------------------
 *
 * Centered white modal that installs a skill via
 * `openhuman.skills_install_from_url`. The Rust side fetches a single
 * `SKILL.md` file over HTTPS and writes it into
 * `<workspace>/.openhuman/skills/<slug>/SKILL.md`. URLs are allow-listed
 * (https only, no private/loopback/link-local/multicast/cloud-metadata
 * hosts) and a wall-clock timeout applies (default 60s, max 600s).
 * `github.com/<o>/<r>/blob/<b>/<p>.md` URLs are auto-rewritten to their
 * `raw.githubusercontent.com` equivalents.
 *
 * UI contract:
 *   - Single URL input (https only, must point at a `.md` file) +
 *     optional timeout in seconds.
 *   - While the RPC is in flight we show a "Fetching…" indicator and
 *     disable close / backdrop-dismiss so the caller sees the outcome.
 *   - On success we surface the list of `newWorkflows` (ids that appeared
 *     post-install) plus captured fetch log / parse-warning panes, then
 *     hand the result back to the caller via `onInstalled` so the
 *     parent can refetch the list and auto-select the row.
 *   - On failure we map the Rust error prefix (`invalid url:`,
 *     `unsupported url form:`, `fetch failed:`, `fetch too large:`,
 *     `fetch timed out`, `invalid SKILL.md:`, `skill already installed`,
 *     `write failed:`) to a short human title + hint, and show the raw
 *     message below it for debugging.
 *
 * Design mirrors `CreateSkillModal` — see `.claude/rules/15-settings-modal-system.md`.
 */
import debug from 'debug';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { trackEvent } from '../../services/analytics';
import {
  type InstallWorkflowFromUrlResult,
  skillsApi,
  type WorkflowSummary,
} from '../../services/api/skillsApi';
import { ModalShell, Spinner, TextField } from '../ui';
import Button from '../ui/Button';

const log = debug('skills:install-dialog');

const INSTALL_FORM_ID = 'install-skill-form';

interface Props {
  onClose: () => void;
  /**
   * Fires when the backend reports the install succeeded. The parent is
   * responsible for refetching the skills list (the RPC already returns
   * the freshly-added ids, but the caller may want full `WorkflowSummary`
   * rows). `newWorkflows` lists ids that appeared post-install.
   */
  onInstalled: (result: InstallWorkflowFromUrlResult) => void;
  /**
   * Optional: used only for symmetry with `CreateSkillModal`. When
   * supplied and the caller wants to auto-open the detail drawer for a
   * specific skill, they can resolve the full `WorkflowSummary` and call
   * this directly. Not invoked by the dialog itself.
   */
  onSelectSkill?: (skill: WorkflowSummary) => void;
}

/**
 * Cheap pre-flight URL shape check — mirrors the hard rules the Rust
 * side enforces so we can fail fast without a round-trip. The Rust
 * side is still authoritative.
 */
function isLikelyValidUrl(raw: string): boolean {
  if (!raw.trim()) return false;
  try {
    const u = new URL(raw.trim());
    return u.protocol === 'https:';
  } catch {
    return false;
  }
}

interface CategorizedError {
  title: string;
  hint: string;
}

/**
 * Map the stable Rust error prefixes from `install_skill_from_url` to a
 * short human-readable title + hint. See
 * `src/openhuman/skills/ops.rs::install_skill_from_url` for the full list.
 */
function categorizeInstallError(
  t: (key: string, fallback?: string) => string,
  raw: string
): CategorizedError {
  const msg = raw.trim();
  const lower = msg.toLowerCase();
  if (lower.startsWith('invalid url:')) {
    return {
      title: t('skills.install.errors.invalidUrlTitle'),
      hint: t('skills.install.errors.invalidUrlHint'),
    };
  }
  if (lower.startsWith('unsupported url form:')) {
    return {
      title: t('skills.install.errors.unsupportedUrlTitle'),
      hint: t('skills.install.errors.unsupportedUrlHint'),
    };
  }
  if (lower.startsWith('fetch too large:')) {
    return {
      title: t('skills.install.errors.fetchTooLargeTitle'),
      hint: t('skills.install.errors.fetchTooLargeHint'),
    };
  }
  if (lower.startsWith('fetch timed out')) {
    return {
      title: t('skills.install.errors.fetchTimedOutTitle'),
      hint: t('skills.install.errors.fetchTimedOutHint'),
    };
  }
  if (lower.startsWith('fetch failed:')) {
    return {
      title: t('skills.install.errors.fetchFailedTitle'),
      hint: t('skills.install.errors.fetchFailedHint'),
    };
  }
  if (lower.startsWith('invalid skill.md:')) {
    return {
      title: t('skills.install.errors.invalidSkillTitle'),
      hint: t('skills.install.errors.invalidSkillHint'),
    };
  }
  if (lower.startsWith('skill already installed')) {
    return {
      title: t('skills.install.errors.alreadyInstalledTitle'),
      hint: t('skills.install.errors.alreadyInstalledHint'),
    };
  }
  if (lower.startsWith('write failed:')) {
    return {
      title: t('skills.install.errors.writeFailedTitle'),
      hint: t('skills.install.errors.writeFailedHint'),
    };
  }
  return {
    title: t('skills.install.errors.genericTitle'),
    hint: t('skills.install.errors.genericHint'),
  };
}

export default function InstallSkillDialog({ onClose, onInstalled }: Props) {
  const { t } = useT();
  const [url, setUrl] = useState('');
  const [timeoutSecs, setTimeoutSecs] = useState<string>('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<InstallWorkflowFromUrlResult | null>(null);

  const firstFieldRef = useRef<HTMLInputElement | null>(null);

  const urlValid = useMemo(() => isLikelyValidUrl(url), [url]);
  const timeoutValid = useMemo(() => {
    if (!timeoutSecs.trim()) return true;
    const n = Number(timeoutSecs);
    return Number.isInteger(n) && n > 0 && n <= 600;
  }, [timeoutSecs]);
  const formValid = urlValid && timeoutValid && !submitting && !result;

  useEffect(() => {
    const raf = window.requestAnimationFrame(() => {
      firstFieldRef.current?.focus();
    });
    log('mount');
    return () => {
      window.cancelAnimationFrame(raf);
      log('unmount');
    };
  }, []);

  const handleSubmit = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      if (!formValid) return;

      const payload = {
        url: url.trim(),
        ...(timeoutSecs.trim() ? { timeoutSecs: Number(timeoutSecs) } : {}),
      };
      log('submit url=%s timeout=%s', payload.url, payload.timeoutSecs ?? 'default');
      setSubmitting(true);
      setError(null);
      try {
        const installed = await skillsApi.installWorkflowFromUrl(payload);
        log(
          'submit-ok new=%d stdout=%d stderr=%d',
          installed.newWorkflows.length,
          installed.stdout.length,
          installed.stderr.length
        );
        for (const skillId of installed.newWorkflows) {
          trackEvent('skill_install', { skill_id: skillId });
        }
        setResult(installed);
        onInstalled(installed);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        log('submit-err %s', message);
        setError(message);
      } finally {
        setSubmitting(false);
      }
    },
    [formValid, onInstalled, timeoutSecs, url]
  );

  return (
    <ModalShell
      onClose={() => {
        if (submitting) return;
        log('close-request');
        onClose();
      }}
      title={t('skills.install.title')}
      titleId="install-skill-title"
      subtitle={
        <>
          {t('skills.install.subtitlePrefix')} <code className="font-mono">SKILL.md</code>{' '}
          {t('skills.install.subtitleMiddle')}{' '}
          <code className="font-mono">.openhuman/skills/</code>.{' '}
          {t('skills.install.subtitleSuffix')}
        </>
      }
      maxWidthClassName="max-w-[560px]"
      contentClassName="max-h-[70vh] overflow-y-auto px-5 py-4"
      closePolicy={submitting ? { escape: false, backdrop: false, button: false } : undefined}
      footer={
        <div className="flex items-center justify-end gap-2">
          <Button variant="tertiary" onClick={onClose} disabled={submitting}>
            {result ? t('common.finish') : t('common.cancel')}
          </Button>
          {result ? null : (
            <Button type="submit" form={INSTALL_FORM_ID} variant="primary" disabled={!formValid}>
              {submitting ? t('skills.install.installing') : t('skills.install.installBtn')}
            </Button>
          )}
        </div>
      }>
      <form id={INSTALL_FORM_ID} onSubmit={handleSubmit} className="space-y-4">
            {/* URL */}
            <div>
              <label
                htmlFor="install-skill-url"
                className="block text-xs font-medium text-content-secondary">
                {t('skills.install.urlLabel')}
                <span className="text-coral-500"> *</span>
              </label>
              <TextField
                id="install-skill-url"
                ref={firstFieldRef}
                type="url"
                mono
                inputMode="url"
                autoComplete="off"
                value={url}
                onChange={e => setUrl(e.target.value)}
                disabled={submitting || result !== null}
                required
                maxLength={2048}
                className="mt-1 shadow-xs disabled:text-content-muted"
                placeholder={t('skills.install.urlPlaceholder')}
              />
              {url.trim() && !urlValid ? (
                <p className="mt-1 text-[11px] text-coral-600">
                  {t('skills.install.urlInvalidPrefix')} <code className="font-mono">https://</code>{' '}
                  {t('skills.install.urlInvalidSuffix')}
                </p>
              ) : (
                <p className="mt-1 text-[11px] text-content-muted">
                  {t('skills.install.urlHelpPrefix')} <code className="font-mono">.md</code>{' '}
                  {t('skills.install.urlHelpMiddle')}{' '}
                  <code className="font-mono">github.com/…/blob/…</code>{' '}
                  {t('skills.install.urlHelpSuffix')}
                  <code className="font-mono"> raw.githubusercontent.com</code>.
                </p>
              )}
            </div>

            {/* Timeout */}
            <div>
              <label
                htmlFor="install-skill-timeout"
                className="block text-xs font-medium text-content-secondary">
                {t('skills.install.timeoutLabel')}
                <span className="ml-1 font-normal text-content-faint">
                  {t('skills.install.timeoutHint')}
                </span>
              </label>
              <TextField
                id="install-skill-timeout"
                type="number"
                inputMode="numeric"
                min={1}
                max={600}
                value={timeoutSecs}
                onChange={e => setTimeoutSecs(e.target.value)}
                disabled={submitting || result !== null}
                className="mt-1 shadow-xs disabled:text-content-muted"
                placeholder={t('skills.install.timeoutPlaceholder')}
              />
              {!timeoutValid ? (
                <p className="mt-1 text-[11px] text-coral-600">
                  {t('skills.install.timeoutInvalid')}
                </p>
              ) : (
                <p className="mt-1 text-[11px] text-content-muted">
                  {t('skills.install.timeoutHelp')}
                </p>
              )}
            </div>

            {/* In-flight indicator */}
            {submitting ? (
              <div
                role="status"
                aria-live="polite"
                className="flex items-center gap-3 rounded-xl border border-primary-200 bg-primary-50 p-3 text-xs text-primary-900">
                <Spinner aria-hidden="true" className="h-3 w-3 shrink-0" />
                <span>
                  {t('skills.install.fetchingPrefix')} <code className="font-mono">SKILL.md</code>…{' '}
                  {t('skills.install.fetchingSuffix')}
                </span>
              </div>
            ) : null}

            {/* Success panel */}
            {result ? (
              <div
                role="status"
                aria-live="polite"
                className="space-y-3 rounded-xl border border-sage-200 bg-sage-50 p-3 text-xs text-sage-900">
                <div>
                  <p className="font-semibold">{t('skills.install.installComplete')}</p>
                  <p className="mt-1">
                    {result.newWorkflows.length > 0
                      ? t('skills.install.successDiscovered').replace(
                          '{count}',
                          String(result.newWorkflows.length)
                        )
                      : t('skills.install.successNoNewIds')}
                  </p>
                  {result.newWorkflows.length > 0 ? (
                    <ul className="mt-1 list-disc pl-5 font-mono">
                      {result.newWorkflows.map(id => (
                        <li key={id}>{id}</li>
                      ))}
                    </ul>
                  ) : null}
                </div>
                {result.stdout ? (
                  <details>
                    <summary className="cursor-pointer font-semibold">
                      {t('skills.install.fetchLog')}
                    </summary>
                    <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded border border-sage-100 bg-surface p-2 font-mono text-[11px] text-content">
                      {result.stdout}
                    </pre>
                  </details>
                ) : null}
                {result.stderr ? (
                  <details>
                    <summary className="cursor-pointer font-semibold">
                      {t('skills.install.parseWarnings')}
                    </summary>
                    <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded border border-sage-100 bg-surface p-2 font-mono text-[11px] text-content">
                      {result.stderr}
                    </pre>
                  </details>
                ) : null}
              </div>
            ) : null}

            {/* Error panel */}
            {error ? (
              <div
                role="alert"
                className="space-y-2 rounded-xl border border-coral-200 bg-coral-50 p-3 text-xs text-coral-900">
                {(() => {
                  const cat = categorizeInstallError(t, error);
                  return (
                    <>
                      <p className="font-semibold">{cat.title}</p>
                      <p>{cat.hint}</p>
                      <details>
                        <summary className="cursor-pointer font-semibold">
                          {t('skills.install.rawError')}
                        </summary>
                        <pre className="mt-1 whitespace-pre-wrap rounded border border-coral-200 bg-surface p-2 font-mono text-[11px] text-content">
                          {error}
                        </pre>
                      </details>
                    </>
                  );
                })()}
              </div>
            ) : null}
      </form>
    </ModalShell>
  );
}
