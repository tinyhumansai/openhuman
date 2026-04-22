/**
 * SkillResourcePreview
 * --------------------
 *
 * Size-gated text viewer for a single SKILL bundled resource. Fetches content
 * via `skillsApi.readSkillResource`. The backend caps payloads at 128 KB, emits
 * a traversal/symlink error as a plain string, and never streams — so the
 * preview pane only has three visual states: loading, error, success.
 *
 * Errors (e.g. "path escape", ">128KB") are surfaced verbatim in a coral
 * panel per the crypto-community design system.
 */
import { useEffect, useState } from 'react';
import debug from 'debug';

import { skillsApi } from '../../services/api/skillsApi';

const log = debug('skills:resource-preview');

interface Props {
  skillId: string;
  relativePath: string;
  onDismiss: () => void;
}

interface LoadState {
  status: 'loading' | 'success' | 'error';
  content?: string;
  bytes?: number;
  error?: string;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

export default function SkillResourcePreview({ skillId, relativePath, onDismiss }: Props) {
  const [state, setState] = useState<LoadState>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;
    log('fetch skillId=%s path=%s', skillId, relativePath);
    skillsApi
      .readSkillResource({ skillId, relativePath })
      .then(result => {
        if (cancelled) return;
        log('success bytes=%d', result.bytes);
        setState({
          status: 'success',
          content: result.content,
          bytes: result.bytes,
        });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        const message = err instanceof Error ? err.message : String(err);
        log('error message=%s', message);
        setState({ status: 'error', error: message });
      });
    return () => {
      cancelled = true;
    };
  }, [skillId, relativePath]);

  return (
    <div className="overflow-hidden rounded-xl border border-stone-200 bg-white shadow-soft">
      <div className="flex items-center justify-between gap-2 border-b border-stone-200 bg-stone-50 px-3 py-2">
        <div className="min-w-0 flex-1">
          <p
            className="truncate font-mono text-[11px] text-stone-700"
            title={relativePath}>
            {relativePath}
          </p>
        </div>
        <button
          type="button"
          onClick={() => {
            log('dismiss skillId=%s path=%s', skillId, relativePath);
            onDismiss();
          }}
          aria-label="Close preview"
          className="flex h-6 w-6 flex-shrink-0 items-center justify-center rounded-md text-stone-400 transition-colors hover:bg-stone-100 hover:text-stone-700 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-1">
          <svg className="h-3.5 w-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M6 18L18 6M6 6l12 12"
            />
          </svg>
        </button>
      </div>

      {state.status === 'loading' ? (
        <div className="flex items-center justify-center gap-2 px-3 py-6 text-xs text-stone-500">
          <svg
            className="h-4 w-4 animate-spin text-primary-500"
            fill="none"
            viewBox="0 0 24 24"
            role="status"
            aria-label="Loading">
            <circle
              className="opacity-25"
              cx="12"
              cy="12"
              r="10"
              stroke="currentColor"
              strokeWidth="4"
            />
            <path
              className="opacity-75"
              fill="currentColor"
              d="M4 12a8 8 0 018-8v4a4 4 0 00-4 4H4z"
            />
          </svg>
          <span>Loading preview…</span>
        </div>
      ) : null}

      {state.status === 'error' ? (
        <div className="border-t border-coral-200 bg-coral-50 px-3 py-3">
          <p className="text-[11px] font-semibold uppercase tracking-wide text-coral-900">
            Preview failed
          </p>
          <p className="mt-1 break-words font-mono text-[11px] leading-relaxed text-coral-800">
            {state.error}
          </p>
        </div>
      ) : null}

      {state.status === 'success' ? (
        <>
          <pre className="max-h-[320px] overflow-auto px-3 py-3 font-mono text-[11px] leading-relaxed text-stone-900 whitespace-pre-wrap break-words">
            {state.content}
          </pre>
          <div className="flex items-center justify-end border-t border-stone-200 bg-stone-50 px-3 py-1.5">
            <span className="text-[10px] font-mono text-stone-500">
              {typeof state.bytes === 'number' ? formatBytes(state.bytes) : ''}
            </span>
          </div>
        </>
      ) : null}
    </div>
  );
}
