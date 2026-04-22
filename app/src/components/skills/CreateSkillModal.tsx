/**
 * CreateSkillModal
 * ----------------
 *
 * Centered white modal that scaffolds a new SKILL.md skill via the
 * `openhuman.skills_create` JSON-RPC method. Matches the settings-modal
 * design rules (clean white, 520px desktop, 16px radius, backdrop + blur,
 * Escape/click-out to close, focus capture) — see
 * `.claude/rules/15-settings-modal-system.md`.
 *
 * Form fields mirror `SkillsCreateParams` on the Rust side:
 *   - name          (required) — display name; also slugified into the
 *                   on-disk skill directory. A live preview surfaces the
 *                   slug so users can see what will hit the filesystem.
 *   - description   (required) — short prose; persisted as the
 *                   `description:` field in the generated YAML frontmatter.
 *   - scope         (user | project) — where SKILL.md is written. The UI
 *                   hides the `legacy` scope since that layout is read-only
 *                   and being phased out.
 *   - license       (optional) — free-form SPDX string (e.g. `MIT`,
 *                   `Apache-2.0`). Forwarded verbatim.
 *   - tags          (optional, CSV) — normalized client-side into an array;
 *                   empty entries are dropped.
 *   - allowedTools  (optional, CSV) — rekeyed to `allowed-tools` on the
 *                   wire by `skillsApi.createSkill`.
 *
 * On success `onCreated(skill)` fires with the freshly-discovered
 * `SkillSummary` so the parent grid can insert the new row without a
 * full refetch. On failure the Rust error string is surfaced verbatim
 * at the bottom of the form and the submit button re-enables.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import debug from 'debug';

import {
  skillsApi,
  type CreateSkillInput,
  type SkillScope,
  type SkillSummary,
} from '../../services/api/skillsApi';

const log = debug('skills:create-modal');

interface Props {
  onClose: () => void;
  onCreated: (skill: SkillSummary) => void;
}

const INITIAL_SCOPE: SkillScope = 'user';

/**
 * Client-side slug preview — mirrors the Rust `slugify_skill_name`
 * heuristic (lowercase, ASCII alphanumerics + `-`, collapse repeats,
 * trim hyphens at the edges). The preview is advisory only; the Rust
 * side is authoritative when the skill is persisted.
 */
function previewSlug(name: string): string {
  const lower = name.normalize('NFKD').toLowerCase();
  let out = '';
  let prevHyphen = false;
  for (const ch of lower) {
    // ASCII alnum pass-through
    if ((ch >= 'a' && ch <= 'z') || (ch >= '0' && ch <= '9')) {
      out += ch;
      prevHyphen = false;
      continue;
    }
    if ((ch === '-' || ch === '_' || /\s/.test(ch)) && !prevHyphen) {
      out += '-';
      prevHyphen = true;
    }
  }
  // Trim leading/trailing hyphens
  return out.replace(/^-+|-+$/g, '');
}

function splitCsv(raw: string): string[] {
  return raw
    .split(',')
    .map(s => s.trim())
    .filter(s => s.length > 0);
}

export default function CreateSkillModal({ onClose, onCreated }: Props) {
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [scope, setScope] = useState<SkillScope>(INITIAL_SCOPE);
  const [license, setLicense] = useState('');
  const [author, setAuthor] = useState('');
  const [tagsCsv, setTagsCsv] = useState('');
  const [allowedToolsCsv, setAllowedToolsCsv] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const firstFieldRef = useRef<HTMLInputElement | null>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  const slug = useMemo(() => previewSlug(name), [name]);

  const nameValid = slug.length > 0;
  const descriptionValid = description.trim().length > 0;
  const formValid = nameValid && descriptionValid && !submitting;

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
      if (!formValid) {
        return;
      }
      const payload: CreateSkillInput = {
        name: name.trim(),
        description: description.trim(),
        scope,
      };
      if (license.trim()) payload.license = license.trim();
      if (author.trim()) payload.author = author.trim();
      const tags = splitCsv(tagsCsv);
      if (tags.length > 0) payload.tags = tags;
      const allowedTools = splitCsv(allowedToolsCsv);
      if (allowedTools.length > 0) payload.allowedTools = allowedTools;

      log('submit name=%s scope=%s', payload.name, payload.scope);
      setSubmitting(true);
      setError(null);
      try {
        const created = await skillsApi.createSkill(payload);
        log('submit-ok id=%s', created.id);
        onCreated(created);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        log('submit-err %s', message);
        setError(message);
        setSubmitting(false);
      }
    },
    [allowedToolsCsv, author, description, formValid, license, name, onCreated, scope, tagsCsv]
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
        className="absolute inset-0 bg-black/50 backdrop-blur-sm animate-fade-in"
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
        aria-labelledby="create-skill-title"
        className="relative w-full max-w-[520px] rounded-2xl bg-white shadow-2xl animate-fade-in">
        <form onSubmit={handleSubmit}>
          {/* Header */}
          <div className="flex items-start justify-between gap-3 border-b border-stone-100 px-5 py-4">
            <div className="min-w-0 flex-1">
              <h2
                id="create-skill-title"
                className="text-base font-semibold text-stone-900 font-sans">
                New skill
              </h2>
              <p className="mt-0.5 text-xs text-stone-500">
                Scaffolds a <code className="font-mono">SKILL.md</code> with the supplied
                frontmatter.
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
            {/* Name */}
            <div>
              <label
                htmlFor="create-skill-name"
                className="block text-xs font-medium text-stone-600">
                Name<span className="text-coral-500"> *</span>
              </label>
              <input
                id="create-skill-name"
                ref={firstFieldRef}
                type="text"
                value={name}
                onChange={e => setName(e.target.value)}
                required
                maxLength={128}
                className="mt-1 w-full rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 shadow-sm transition-colors focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/30"
                placeholder="e.g. Trade Journal"
              />
              <p className="mt-1 text-[11px] text-stone-500">
                Slug:{' '}
                <code className="rounded bg-stone-100 px-1 py-[1px] font-mono text-stone-700">
                  {slug || '—'}
                </code>
              </p>
            </div>

            {/* Description */}
            <div>
              <label
                htmlFor="create-skill-description"
                className="block text-xs font-medium text-stone-600">
                Description<span className="text-coral-500"> *</span>
              </label>
              <textarea
                id="create-skill-description"
                value={description}
                onChange={e => setDescription(e.target.value)}
                required
                rows={3}
                maxLength={500}
                className="mt-1 w-full rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 shadow-sm transition-colors focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/30"
                placeholder="What does this skill do?"
              />
            </div>

            {/* Scope */}
            <fieldset>
              <legend className="block text-xs font-medium text-stone-600">Scope</legend>
              <div className="mt-1 flex gap-2">
                {(['user', 'project'] as const).map(s => {
                  const selected = scope === s;
                  return (
                    <label
                      key={s}
                      className={`flex flex-1 cursor-pointer items-center gap-2 rounded-lg border px-3 py-2 text-sm transition-colors ${
                        selected
                          ? 'border-primary-500 bg-primary-50 text-primary-900'
                          : 'border-stone-200 bg-white text-stone-700 hover:border-stone-300'
                      }`}>
                      <input
                        type="radio"
                        name="create-skill-scope"
                        value={s}
                        checked={selected}
                        onChange={() => setScope(s)}
                        className="h-3 w-3 accent-primary-500"
                      />
                      <span className="capitalize">{s}</span>
                    </label>
                  );
                })}
              </div>
              <p className="mt-1 text-[11px] text-stone-500">
                {scope === 'user'
                  ? 'Written to ~/.openhuman/skills/<slug>/SKILL.md — available across all workspaces.'
                  : 'Written to <workspace>/.openhuman/skills/<slug>/SKILL.md — requires workspace trust.'}
              </p>
            </fieldset>

            {/* License / Author */}
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <div>
                <label
                  htmlFor="create-skill-license"
                  className="block text-xs font-medium text-stone-600">
                  License
                </label>
                <input
                  id="create-skill-license"
                  type="text"
                  value={license}
                  onChange={e => setLicense(e.target.value)}
                  maxLength={64}
                  className="mt-1 w-full rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 shadow-sm transition-colors focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/30"
                  placeholder="MIT"
                />
              </div>
              <div>
                <label
                  htmlFor="create-skill-author"
                  className="block text-xs font-medium text-stone-600">
                  Author
                </label>
                <input
                  id="create-skill-author"
                  type="text"
                  value={author}
                  onChange={e => setAuthor(e.target.value)}
                  maxLength={128}
                  className="mt-1 w-full rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 shadow-sm transition-colors focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/30"
                  placeholder="Your name"
                />
              </div>
            </div>

            {/* Tags */}
            <div>
              <label
                htmlFor="create-skill-tags"
                className="block text-xs font-medium text-stone-600">
                Tags
                <span className="ml-1 font-normal text-stone-400">(comma-separated)</span>
              </label>
              <input
                id="create-skill-tags"
                type="text"
                value={tagsCsv}
                onChange={e => setTagsCsv(e.target.value)}
                maxLength={256}
                className="mt-1 w-full rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 shadow-sm transition-colors focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/30"
                placeholder="trading, research"
              />
            </div>

            {/* Allowed tools */}
            <div>
              <label
                htmlFor="create-skill-tools"
                className="block text-xs font-medium text-stone-600">
                Allowed tools
                <span className="ml-1 font-normal text-stone-400">(comma-separated)</span>
              </label>
              <input
                id="create-skill-tools"
                type="text"
                value={allowedToolsCsv}
                onChange={e => setAllowedToolsCsv(e.target.value)}
                maxLength={512}
                className="mt-1 w-full rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm font-mono text-stone-900 shadow-sm transition-colors focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/30"
                placeholder="node_exec, fetch"
              />
              <p className="mt-1 text-[11px] text-stone-500">
                Rendered into the SKILL.md frontmatter as{' '}
                <code className="font-mono">allowed-tools:</code>.
              </p>
            </div>

            {/* Error */}
            {error ? (
              <div
                role="alert"
                className="rounded-xl border border-coral-200 bg-coral-50 p-3 text-xs text-coral-900">
                <p className="font-semibold">Could not create skill</p>
                <p className="mt-1 whitespace-pre-wrap font-mono">{error}</p>
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
              Cancel
            </button>
            <button
              type="submit"
              disabled={!formValid}
              className="rounded-lg bg-primary-500 px-4 py-2 text-sm font-semibold text-white transition-colors hover:bg-primary-600 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1 disabled:cursor-not-allowed disabled:opacity-50">
              {submitting ? 'Creating…' : 'Create skill'}
            </button>
          </div>
        </form>
      </div>
    </div>,
    document.body
  );
}
