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
 *   - On success we surface the list of `newSkills` (ids that appeared
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
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import debug from 'debug';

import {
  skillsApi,
  type InstallSkillFromUrlResult,
  type SkillSummary,
} from '../../services/api/skillsApi';
import { trackEvent } from '../../services/analytics';

const log = debug('skills:install-dialog');

interface Props {
  onClose: () => void;
  /**
   * Fires when the backend reports the install succeeded. The parent is
   * responsible for refetching the skills list (the RPC already returns
   * the freshly-added ids, but the caller may want full `SkillSummary`
   * rows). `newSkills` lists ids that appeared post-install.
   */
  onInstalled: (result: InstallSkillFromUrlResult) => void;
  /**
   * Optional: used only for symmetry with `CreateSkillModal`. When
   * supplied and the caller wants to auto-open the detail drawer for a
   * specific skill, they can resolve the full `SkillSummary` and call
   * this directly. Not invoked by the dialog itself.
   */
  onSelectSkill?: (skill: SkillSummary) => void;
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
function categorizeInstallError(raw: string): CategorizedError {
  const msg = raw.trim();
  const lower = msg.toLowerCase();
  if (lower.startsWith('invalid url:')) {
    return {
      title: 'URL rejected',
      hint: 'Only public HTTPS URLs are allowed. Private, loopback, and metadata hosts are blocked.',
    };
  }
  if (lower.startsWith('unsupported url form:')) {
    return {
      title: 'URL form not supported',
      hint: 'Only direct `.md` links work. For GitHub, link to a file (github.com/owner/repo/blob/…/SKILL.md) — tree and repo roots are not installed.',
    };
  }
  if (lower.startsWith('fetch too large:')) {
    return {
      title: 'SKILL.md too large',
      hint: 'The SKILL.md must be under 1 MiB. Split bundled resources into `references/` or `scripts/` files instead of inlining them.',
    };
  }
  if (lower.startsWith('fetch timed out')) {
    return {
      title: 'Fetch timed out',
      hint: 'The remote host did not respond in time. Try again or raise the timeout (1–600 s).',
    };
  }
  if (lower.startsWith('fetch failed:')) {
    return {
      title: 'Fetch failed',
      hint: 'The request did not complete successfully. Check the URL points at a reachable public file, and that the host returned a 2xx response.',
    };
  }
  if (lower.startsWith('invalid skill.md:')) {
    return {
      title: 'SKILL.md did not parse',
      hint: 'The frontmatter must be valid YAML with non-empty `name` and `description` fields, terminated by `---`.',
    };
  }
  if (lower.startsWith('skill already installed')) {
    return {
      title: 'Skill already installed',
      hint: 'A skill with this slug already exists in the workspace. Remove it first or change the frontmatter `metadata.id` / `name`.',
    };
  }
  if (lower.startsWith('write failed:')) {
    return {
      title: 'Could not write SKILL.md',
      hint: 'The workspace skills directory was not writable. Check filesystem permissions for `<workspace>/.openhuman/skills/`.',
    };
  }
  return {
    title: 'Could not install skill',
    hint: 'The backend returned an error. The raw message is shown below.',
  };
}

export default function InstallSkillDialog({ onClose, onInstalled }: Props) {
  const [url, setUrl] = useState('');
  const [timeoutSecs, setTimeoutSecs] = useState<string>('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<InstallSkillFromUrlResult | null>(null);

  const firstFieldRef = useRef<HTMLInputElement | null>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  const urlValid = useMemo(() => isLikelyValidUrl(url), [url]);
  const timeoutValid = useMemo(() => {
    if (!timeoutSecs.trim()) return true;
    const n = Number(timeoutSecs);
    return Number.isInteger(n) && n > 0 && n <= 600;
  }, [timeoutSecs]);
  const formValid = urlValid && timeoutValid && !submitting && !result;

  useEffect(() => {
    previousFocusRef.current = document.activeElement as HTMLElement | null;
    const raf = window.requestAnimationFrame(() => {
      firstFieldRef.current?.focus();
    });
    log('mount');
    return () => {
      window.cancelAnimationFrame(raf);
      previousFocusRef.current?.focus?.();
      log('unmount');
    };
  }, []);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !submitting) {
        log('escape-key close');
        onClose();
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [onClose, submitting]);

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
        const installed = await skillsApi.installSkillFromUrl(payload);
        log(
          'submit-ok new=%d stdout=%d stderr=%d',
          installed.newSkills.length,
          installed.stdout.length,
          installed.stderr.length
        );
        for (const skillId of installed.newSkills) {
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

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
      onClick={e => {
        if (e.target === e.currentTarget && !submitting) {
          log('backdrop-click close');
          onClose();
        }
      }}>
      <div
        aria-hidden="true"
        className="absolute inset-0 animate-fade-in bg-black/50 backdrop-blur-sm"
        onClick={() => {
          if (!submitting) {
            log('backdrop-direct close');
            onClose();
          }
        }}
      />

      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="install-skill-title"
        className="relative w-full max-w-[560px] animate-fade-in rounded-2xl bg-white shadow-2xl">
        <form onSubmit={handleSubmit}>
          {/* Header */}
          <div className="flex items-start justify-between gap-3 border-b border-stone-100 px-5 py-4">
            <div className="min-w-0 flex-1">
              <h2
                id="install-skill-title"
                className="font-sans text-base font-semibold text-stone-900">
                Install skill from URL
              </h2>
              <p className="mt-0.5 text-xs text-stone-500">
                Fetches a single <code className="font-mono">SKILL.md</code> over HTTPS and installs
                it under <code className="font-mono">.openhuman/skills/</code>. HTTPS only; private
                and loopback hosts are blocked.
              </p>
            </div>
            <button
              type="button"
              onClick={() => {
                if (!submitting) {
                  log('close-button');
                  onClose();
                }
              }}
              disabled={submitting}
              aria-label="Close"
              className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-lg text-stone-400 transition-colors hover:bg-stone-100 hover:text-stone-600 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1 disabled:opacity-40">
              <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M6 18L18 6M6 6l12 12"
                />
              </svg>
            </button>
          </div>

          {/* Body */}
          <div className="max-h-[70vh] space-y-4 overflow-y-auto px-5 py-4">
            {/* URL */}
            <div>
              <label
                htmlFor="install-skill-url"
                className="block text-xs font-medium text-stone-600">
                Skill URL<span className="text-coral-500"> *</span>
              </label>
              <input
                id="install-skill-url"
                ref={firstFieldRef}
                type="url"
                inputMode="url"
                autoComplete="off"
                value={url}
                onChange={e => setUrl(e.target.value)}
                disabled={submitting || result !== null}
                required
                maxLength={2048}
                className="mt-1 w-full rounded-lg border border-stone-200 bg-white px-3 py-2 font-mono text-sm text-stone-900 shadow-sm transition-colors focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/30 disabled:bg-stone-50 disabled:text-stone-500"
                placeholder="https://raw.githubusercontent.com/owner/repo/main/SKILL.md"
              />
              {url.trim() && !urlValid ? (
                <p className="mt-1 text-[11px] text-coral-600">
                  URL must be a well-formed <code className="font-mono">https://</code> link.
                </p>
              ) : (
                <p className="mt-1 text-[11px] text-stone-500">
                  Direct link to a <code className="font-mono">.md</code> file.{' '}
                  <code className="font-mono">github.com/…/blob/…</code> URLs auto-rewrite to
                  <code className="font-mono"> raw.githubusercontent.com</code>.
                </p>
              )}
            </div>

            {/* Timeout */}
            <div>
              <label
                htmlFor="install-skill-timeout"
                className="block text-xs font-medium text-stone-600">
                Timeout
                <span className="ml-1 font-normal text-stone-400">(seconds, optional)</span>
              </label>
              <input
                id="install-skill-timeout"
                type="number"
                inputMode="numeric"
                min={1}
                max={600}
                value={timeoutSecs}
                onChange={e => setTimeoutSecs(e.target.value)}
                disabled={submitting || result !== null}
                className="mt-1 w-full rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 shadow-sm transition-colors focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/30 disabled:bg-stone-50 disabled:text-stone-500"
                placeholder="60"
              />
              {!timeoutValid ? (
                <p className="mt-1 text-[11px] text-coral-600">Must be an integer between 1 and 600.</p>
              ) : (
                <p className="mt-1 text-[11px] text-stone-500">
                  Defaults to 60 seconds. Values outside 1–600 are clamped server-side.
                </p>
              )}
            </div>

            {/* In-flight indicator */}
            {submitting ? (
              <div
                role="status"
                aria-live="polite"
                className="flex items-center gap-3 rounded-xl border border-primary-200 bg-primary-50 p-3 text-xs text-primary-900">
                <span
                  aria-hidden="true"
                  className="h-3 w-3 flex-shrink-0 animate-spin rounded-full border-2 border-primary-300 border-t-primary-600"
                />
                <span>
                  Fetching <code className="font-mono">SKILL.md</code>… this can take up to the
                  timeout you configured.
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
                  <p className="font-semibold">Install complete</p>
                  <p className="mt-1">
                    {result.newSkills.length > 0
                      ? `Discovered ${result.newSkills.length} new skill${result.newSkills.length === 1 ? '' : 's'}.`
                      : 'Skill installed, but no new skill ids appeared — the catalog may already contain a skill with the same slug.'}
                  </p>
                  {result.newSkills.length > 0 ? (
                    <ul className="mt-1 list-disc pl-5 font-mono">
                      {result.newSkills.map(id => (
                        <li key={id}>{id}</li>
                      ))}
                    </ul>
                  ) : null}
                </div>
                {result.stdout ? (
                  <details>
                    <summary className="cursor-pointer font-semibold">Fetch log</summary>
                    <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded border border-sage-100 bg-white p-2 font-mono text-[11px] text-stone-800">
                      {result.stdout}
                    </pre>
                  </details>
                ) : null}
                {result.stderr ? (
                  <details>
                    <summary className="cursor-pointer font-semibold">Parse warnings</summary>
                    <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded border border-sage-100 bg-white p-2 font-mono text-[11px] text-stone-800">
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
                  const cat = categorizeInstallError(error);
                  return (
                    <>
                      <p className="font-semibold">{cat.title}</p>
                      <p>{cat.hint}</p>
                      <details>
                        <summary className="cursor-pointer font-semibold">Raw error</summary>
                        <pre className="mt-1 whitespace-pre-wrap rounded border border-coral-200 bg-white p-2 font-mono text-[11px] text-stone-800">
                          {error}
                        </pre>
                      </details>
                    </>
                  );
                })()}
              </div>
            ) : null}
          </div>

          {/* Footer */}
          <div className="flex items-center justify-end gap-2 border-t border-stone-100 px-5 py-3">
            <button
              type="button"
              onClick={onClose}
              disabled={submitting}
              className="rounded-lg px-4 py-2 text-sm font-medium text-stone-600 transition-colors hover:bg-stone-100 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1 disabled:opacity-40">
              {result ? 'Done' : 'Cancel'}
            </button>
            {result ? null : (
              <button
                type="submit"
                disabled={!formValid}
                className="rounded-lg bg-primary-500 px-4 py-2 text-sm font-semibold text-white transition-colors hover:bg-primary-600 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1 disabled:cursor-not-allowed disabled:opacity-50">
                {submitting ? 'Installing…' : 'Install'}
              </button>
            )}
          </div>
        </form>
      </div>
    </div>,
    document.body
  );
}
