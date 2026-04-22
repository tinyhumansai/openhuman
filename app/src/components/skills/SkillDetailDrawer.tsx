/**
 * SkillDetailDrawer
 * -----------------
 *
 * Right-side slide-in drawer that surfaces metadata for a discovered SKILL.md
 * skill plus a browsable tree of bundled resources (`scripts/`, `references/`,
 * `assets/`). Clicking a resource loads its contents via
 * `skillsApi.readSkillResource` and renders it in a size-gated preview pane.
 *
 * Accessibility / UX rules (per `.claude/rules/15-settings-modal-system.md`):
 * - Rendered via `createPortal` on `document.body` so it overlays everything.
 * - Backdrop click or Escape closes the drawer.
 * - `role="dialog"` / `aria-modal="true"` / labelled heading.
 * - Focus is captured on open and returned on close.
 * - 520px wide on desktop, slides in from the right in 200ms ease-out.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import debug from 'debug';

import type { SkillSummary } from '../../services/api/skillsApi';
import SkillResourcePreview from './SkillResourcePreview';
import SkillResourceTree from './SkillResourceTree';

const log = debug('skills:drawer');

interface Props {
  skill: SkillSummary;
  onClose: () => void;
}

function scopePill(scope: SkillSummary['scope'], legacy: boolean): { label: string; cls: string } {
  if (legacy) {
    return {
      label: 'Legacy',
      cls: 'bg-stone-100 text-stone-700 border-stone-200',
    };
  }
  switch (scope) {
    case 'user':
      return {
        label: 'User',
        // Sage tones for user-scope per design system.
        cls: 'bg-sage-50 text-sage-700 border-sage-200',
      };
    case 'project':
      return {
        label: 'Project',
        // Amber tones for project-scope (trust-gated surface).
        cls: 'bg-amber-50 text-amber-700 border-amber-200',
      };
    case 'legacy':
    default:
      return {
        label: 'Legacy',
        cls: 'bg-stone-100 text-stone-700 border-stone-200',
      };
  }
}

export default function SkillDetailDrawer({ skill, onClose }: Props) {
  const [selectedResource, setSelectedResource] = useState<string | null>(null);
  const closeBtnRef = useRef<HTMLButtonElement | null>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  // Capture focus on mount, restore on unmount.
  useEffect(() => {
    previousFocusRef.current = document.activeElement as HTMLElement | null;
    // Defer focus grab to next frame so the portal content is attached.
    const raf = window.requestAnimationFrame(() => {
      closeBtnRef.current?.focus();
    });
    log('mount skillId=%s', skill.id);
    return () => {
      window.cancelAnimationFrame(raf);
      previousFocusRef.current?.focus?.();
      log('unmount skillId=%s', skill.id);
    };
  }, [skill.id]);

  // Close on Escape key.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        log('escape-key close skillId=%s', skill.id);
        onClose();
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [onClose, skill.id]);

  const pill = useMemo(() => scopePill(skill.scope, skill.legacy), [skill.scope, skill.legacy]);

  const handleSelect = useCallback(
    (path: string) => {
      log('select-resource skillId=%s path=%s', skill.id, path);
      setSelectedResource(path);
    },
    [skill.id]
  );

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex"
      onClick={e => {
        if (e.target === e.currentTarget) {
          log('backdrop-click close skillId=%s', skill.id);
          onClose();
        }
      }}>
      {/* Backdrop */}
      <div
        aria-hidden="true"
        className="absolute inset-0 bg-black/50 backdrop-blur-sm animate-fade-in"
        onClick={() => {
          log('backdrop-direct close skillId=%s', skill.id);
          onClose();
        }}
      />

      {/* Drawer */}
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="skill-drawer-title"
        className="relative ml-auto flex h-full w-full max-w-[520px] flex-col bg-white shadow-2xl animate-slide-in-right">
        {/* Header */}
        <div className="flex items-start justify-between gap-3 border-b border-stone-100 px-5 py-4">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <h2
                id="skill-drawer-title"
                className="truncate text-base font-semibold text-stone-900 font-sans">
                {skill.name}
              </h2>
              <span
                className={`inline-flex flex-shrink-0 items-center rounded-full border px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide ${pill.cls}`}>
                {pill.label}
              </span>
            </div>
            {skill.version ? (
              <p className="mt-1 text-xs text-stone-500 font-mono">v{skill.version}</p>
            ) : null}
          </div>
          <button
            ref={closeBtnRef}
            type="button"
            onClick={() => {
              log('close-button skillId=%s', skill.id);
              onClose();
            }}
            aria-label="Close skill details"
            className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-lg text-stone-400 transition-colors hover:bg-stone-100 hover:text-stone-600 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1">
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
        <div className="flex-1 overflow-y-auto">
          <div className="px-5 py-4 space-y-4">
            {/* Description */}
            {skill.description ? (
              <p className="text-sm leading-relaxed text-stone-700 font-sans">
                {skill.description}
              </p>
            ) : null}

            {/* Metadata grid */}
            <dl className="grid grid-cols-[auto,1fr] gap-x-3 gap-y-2 text-xs">
              {skill.author ? (
                <>
                  <dt className="font-medium text-stone-500">Author</dt>
                  <dd className="text-stone-800">{skill.author}</dd>
                </>
              ) : null}
              {skill.tags.length > 0 ? (
                <>
                  <dt className="font-medium text-stone-500">Tags</dt>
                  <dd className="flex flex-wrap gap-1">
                    {skill.tags.map(tag => (
                      <span
                        key={tag}
                        className="inline-flex items-center rounded-md border border-stone-200 bg-stone-50 px-1.5 py-0.5 text-[10px] text-stone-700">
                        {tag}
                      </span>
                    ))}
                  </dd>
                </>
              ) : null}
              {skill.tools.length > 0 ? (
                <>
                  <dt className="font-medium text-stone-500">Allowed tools</dt>
                  <dd className="flex flex-wrap gap-1">
                    {skill.tools.map(tool => (
                      <span
                        key={tool}
                        className="inline-flex items-center rounded-md border border-primary-100 bg-primary-50 px-1.5 py-0.5 font-mono text-[10px] text-primary-700">
                        {tool}
                      </span>
                    ))}
                  </dd>
                </>
              ) : null}
              {skill.location ? (
                <>
                  <dt className="font-medium text-stone-500">Location</dt>
                  <dd className="truncate font-mono text-[11px] text-stone-600" title={skill.location}>
                    {skill.location}
                  </dd>
                </>
              ) : null}
            </dl>

            {/* Warnings */}
            {skill.warnings.length > 0 ? (
              <div className="rounded-xl border border-amber-200 bg-amber-50 p-3">
                <p className="text-[11px] font-semibold uppercase tracking-wide text-amber-900">
                  Warnings
                </p>
                <ul className="mt-1.5 list-disc space-y-1 pl-4 text-xs text-amber-800">
                  {skill.warnings.map((w, i) => (
                    <li key={i}>{w}</li>
                  ))}
                </ul>
              </div>
            ) : null}

            {/* Resources */}
            <div>
              <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wide text-stone-500">
                Bundled resources ({skill.resources.length})
              </h3>
              {skill.resources.length === 0 ? (
                <p className="text-xs text-stone-400 italic">No bundled resources.</p>
              ) : (
                <SkillResourceTree
                  resources={skill.resources}
                  selectedPath={selectedResource}
                  onSelect={handleSelect}
                />
              )}
            </div>

            {/* Preview pane */}
            {selectedResource ? (
              <SkillResourcePreview
                key={`${skill.id}:${selectedResource}`}
                skillId={skill.id}
                relativePath={selectedResource}
                onDismiss={() => {
                  log('dismiss-preview skillId=%s', skill.id);
                  setSelectedResource(null);
                }}
              />
            ) : null}
          </div>
        </div>
      </div>
    </div>,
    document.body
  );
}
