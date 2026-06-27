import { useEffect, useMemo, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { listArtifactsForThread } from '../../services/artifactDownloadService';
import { upsertArtifactReadyForThread } from '../../store/chatRuntimeSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import ChatFilesPanel from './ChatFilesPanel';

/**
 * Header chip surfacing the count of ready artifacts for a thread (#3024).
 *
 * - Hidden when the count is zero (no chrome cost on chats that never
 *   generated a file).
 * - Click opens the {@link ChatFilesPanel} popover; clicking again or
 *   pressing Esc closes it (panel owns Esc + click-outside).
 *
 * Reads from the persisted `chatRuntime.artifactsByThread` slice — entries
 * survive app restarts via the `artifactsReadyOnlyTransform` configured in
 * `store/index.ts`.
 */
export interface ChatFilesChipProps {
  threadId: string;
}

export default function ChatFilesChip({ threadId }: ChatFilesChipProps) {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const [open, setOpen] = useState(false);
  const normalizedThreadId = threadId.trim();
  const artifactsByThread = useAppSelector(state => state.chatRuntime.artifactsByThread);
  // Only ready artifacts are listable — in_progress / failed live above the
  // composer until they resolve.
  const readyArtifacts = useMemo(
    () => (artifactsByThread[normalizedThreadId] ?? []).filter(a => a.status === 'ready'),
    [artifactsByThread, normalizedThreadId]
  );
  const count = readyArtifacts.length;

  useEffect(() => {
    if (!normalizedThreadId) return;
    let cancelled = false;
    void listArtifactsForThread(normalizedThreadId).then(outcome => {
      if (cancelled) return;
      if (!outcome.ok) {
        console.warn('[artifact] ChatFilesChip failed to hydrate thread artifacts', {
          threadId: normalizedThreadId,
          error: outcome.error,
        });
        return;
      }
      if (outcome.artifacts.length === 0) return;
      console.debug('[artifact] ChatFilesChip hydrating thread artifacts', {
        threadId: normalizedThreadId,
        count: outcome.artifacts.length,
      });
      for (const artifact of outcome.artifacts) {
        dispatch(
          upsertArtifactReadyForThread({
            threadId: normalizedThreadId,
            artifactId: artifact.artifactId,
            kind: artifact.kind,
            title: artifact.title,
            path: artifact.path,
            sizeBytes: artifact.sizeBytes,
          })
        );
      }
    });
    return () => {
      cancelled = true;
    };
  }, [dispatch, normalizedThreadId]);

  if (count === 0) return null;

  return (
    <div className="relative inline-block">
      <button
        type="button"
        data-analytics-id="chat-files-chip"
        onClick={() => setOpen(prev => !prev)}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={t(
          count === 1 ? 'chat.files.chip.aria.one' : 'chat.files.chip.aria.other'
        ).replace('{count}', String(count))}
        data-testid="chat-files-chip"
        className="h-7 inline-flex items-center gap-1.5 rounded-lg border border-line bg-surface hover:bg-surface-hover text-xs font-medium text-content-secondary transition-colors px-2">
        <svg
          aria-hidden="true"
          className="w-3.5 h-3.5"
          fill="none"
          stroke="currentColor"
          strokeWidth={1.8}
          viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            d="M21.44 11.05l-9.19 9.19a6 6 0 01-8.49-8.49l9.19-9.19a4 4 0 015.66 5.66l-9.2 9.19a2 2 0 01-2.83-2.83l8.49-8.49"
          />
        </svg>
        <span data-testid="chat-files-chip-count" className="font-mono leading-none">
          {count}
        </span>
      </button>
      {open && (
        <ChatFilesPanel
          threadId={normalizedThreadId}
          artifacts={readyArtifacts}
          onClose={() => setOpen(false)}
        />
      )}
    </div>
  );
}
