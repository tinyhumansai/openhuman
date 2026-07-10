import { useEffect, useRef, useState } from 'react';

import { formatFileSize } from '../../lib/attachments';
import { useT } from '../../lib/i18n/I18nContext';
import {
  type ArtifactErrorCode,
  deleteArtifact,
  downloadArtifact,
  revealArtifactInFileManager,
} from '../../services/artifactDownloadService';
import {
  type ArtifactSnapshot,
  removeArtifactForThread,
  upsertArtifactReadyForThread,
} from '../../store/chatRuntimeSlice';
import { useAppDispatch } from '../../store/hooks';
import Button from '../ui/Button';

/**
 * Popover panel listing every `ready` artifact for a thread (#3024).
 *
 * Mounted by {@link ChatFilesChip}. Renders one row per artifact with:
 *  - kind icon + title + human-readable size
 *  - Download (Tauri `download_artifact_to_downloads`)
 *  - Show-in-folder (only after a successful download in this session)
 *  - Delete with a confirm-step (optimistic slice removal + RPC call,
 *    re-upsert on failure with a toast).
 *
 * Closes on Esc + click-outside. Empty state copy is the panel's only
 * fallback (the chip itself is hidden when count is zero, so the empty
 * state only shows if the user deletes the last artifact while the
 * panel is open).
 */
export interface ChatFilesPanelProps {
  threadId: string;
  artifacts: ArtifactSnapshot[];
  onClose: () => void;
}

/**
 * Map a structured {@link ArtifactErrorCode} to a localized headline.
 * Caller passes the raw `outcome.error` as a fallback — if the service
 * returns no code (e.g. older callers), the raw text wins. Routing all
 * codes through `t(...)` keeps non-English locales from leaking English
 * error copy into the panel.
 */
function localizeErrorCode(
  t: (key: string, fallback?: string) => string,
  code: ArtifactErrorCode | undefined,
  fallback: string | undefined
): string {
  if (!code) return fallback ?? '';
  switch (code) {
    case 'NOT_DESKTOP':
      return t('chat.files.error.not_desktop');
    case 'MISSING_ARTIFACT_ID':
      return t('chat.files.error.missing_artifact_id');
    case 'MISSING_ARTIFACT_PATH':
      return t('chat.files.error.missing_artifact_path');
    case 'RESOLVE_FAILED':
      return t('chat.files.error.resolve_failed');
    case 'DOWNLOAD_FAILED':
      return t('chat.files.error.download_failed');
    case 'DELETE_FAILED':
      return t('chat.files.error.delete_failed');
    case 'CANCELLED':
      // User-initiated dialog dismissal — not a real error. Callers treat
      // it as a no-op; surface nothing (fall back to raw text if passed).
      return fallback ?? '';
    default: {
      // Exhaustive guard: a new code added to ArtifactErrorCode without a
      // matching arm here will fail to type-check.
      const _exhaustive: never = code;
      return _exhaustive;
    }
  }
}

function extensionFor(kind: ArtifactSnapshot['kind'], title: string): string {
  const dot = title.lastIndexOf('.');
  if (dot > 0 && dot < title.length - 1) {
    return title.slice(dot + 1).toLowerCase();
  }
  switch (kind) {
    case 'presentation':
      return 'pptx';
    case 'document':
      return 'pdf';
    case 'image':
      return 'png';
    default:
      return 'bin';
  }
}

function KindIcon({ kind }: { kind: ArtifactSnapshot['kind'] }) {
  const stroke = 'currentColor';
  switch (kind) {
    case 'presentation':
      return (
        <svg
          aria-hidden="true"
          className="w-4 h-4 flex-shrink-0"
          fill="none"
          stroke={stroke}
          strokeWidth={1.8}
          viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" d="M3 5h18v12H3z" />
          <path strokeLinecap="round" d="M8 21h8M12 17v4" />
          <path strokeLinecap="round" strokeLinejoin="round" d="M7 11l3 3 4-5 3 4" />
        </svg>
      );
    case 'document':
      return (
        <svg
          aria-hidden="true"
          className="w-4 h-4 flex-shrink-0"
          fill="none"
          stroke={stroke}
          strokeWidth={1.8}
          viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            d="M14 3H7a2 2 0 00-2 2v14a2 2 0 002 2h10a2 2 0 002-2V8z"
          />
          <path strokeLinecap="round" d="M14 3v5h5M9 13h6M9 17h6" />
        </svg>
      );
    case 'image':
      return (
        <svg
          aria-hidden="true"
          className="w-4 h-4 flex-shrink-0"
          fill="none"
          stroke={stroke}
          strokeWidth={1.8}
          viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" d="M3 5h18v14H3z" />
          <circle cx="9" cy="10" r="1.5" />
          <path strokeLinecap="round" strokeLinejoin="round" d="M3 17l5-5 4 4 3-3 6 6" />
        </svg>
      );
    default:
      return (
        <svg
          aria-hidden="true"
          className="w-4 h-4 flex-shrink-0"
          fill="none"
          stroke={stroke}
          strokeWidth={1.8}
          viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" d="M4 4h16v16H4z" />
        </svg>
      );
  }
}

interface RowDownloadState {
  state: 'idle' | 'downloading' | 'done' | 'error';
  path?: string;
  /** Already-localized error headline ready for direct render. */
  error?: string;
}

export default function ChatFilesPanel({ threadId, artifacts, onClose }: ChatFilesPanelProps) {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [downloadState, setDownloadState] = useState<Record<string, RowDownloadState>>({});
  const [deleteError, setDeleteError] = useState<string | null>(null);

  // Esc closes. Use keydown not keyup so a panel that opened via Enter
  // doesn't immediately re-trigger its trigger on the same release.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        if (confirmDeleteId) {
          setConfirmDeleteId(null);
        } else {
          onClose();
        }
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [confirmDeleteId, onClose]);

  // Click-outside closes. Pointerdown fires before click, so the chip's
  // toggle handler doesn't immediately re-open the just-closed panel.
  useEffect(() => {
    const onPointer = (e: PointerEvent) => {
      const node = containerRef.current;
      if (!node) return;
      const target = e.target as Node | null;
      if (target && !node.contains(target)) {
        onClose();
      }
    };
    document.addEventListener('pointerdown', onPointer);
    return () => document.removeEventListener('pointerdown', onPointer);
  }, [onClose]);

  const handleDownload = async (artifact: ArtifactSnapshot) => {
    setDownloadState(prev => ({ ...prev, [artifact.artifactId]: { state: 'downloading' } }));
    const ext = extensionFor(artifact.kind, artifact.title);
    const outcome = await downloadArtifact(artifact.artifactId, artifact.title, ext);
    setDownloadState(prev => ({
      ...prev,
      [artifact.artifactId]: outcome.ok
        ? { state: 'done', path: outcome.path }
        : {
            state: 'error',
            // Prefer the localized headline; only fall back to the raw
            // detail when the service didn't supply a code (defensive —
            // every documented failure path returns one).
            error: localizeErrorCode(t, outcome.code, outcome.error),
          },
    }));
  };

  const handleReveal = async (path: string) => {
    if (path) {
      await revealArtifactInFileManager(path);
    }
  };

  const handleDeleteConfirm = async (artifact: ArtifactSnapshot) => {
    setConfirmDeleteId(null);
    setDeleteError(null);
    // Optimistic remove — re-upsert on failure so the row reappears.
    dispatch(removeArtifactForThread({ threadId, artifactId: artifact.artifactId }));
    const outcome = await deleteArtifact(artifact.artifactId);
    if (!outcome.ok) {
      // Re-insert the snapshot so the user sees it again. Keep updatedAt
      // fresh so the row sorts in the same spot it left.
      dispatch(
        upsertArtifactReadyForThread({
          threadId,
          artifactId: artifact.artifactId,
          kind: artifact.kind,
          title: artifact.title,
          path: artifact.path ?? '',
          sizeBytes: artifact.sizeBytes ?? 0,
        })
      );
      // Prefer the localized headline mapped from `code`; if neither
      // code nor a raw detail came back, fall back to the generic
      // delete-failed copy so the user always sees something.
      setDeleteError(
        localizeErrorCode(t, outcome.code, outcome.error) || t('chat.files.delete.failed')
      );
    }
  };

  return (
    <div
      ref={containerRef}
      role="dialog"
      aria-label={t('chat.files.panel.aria')}
      data-testid="chat-files-panel"
      className="absolute right-0 top-9 z-30 w-[360px] max-h-[420px] overflow-y-auto rounded-xl border border-line bg-surface shadow-lg">
      <header className="sticky top-0 z-10 bg-surface border-b border-line-subtle px-3 py-2 flex items-center justify-between">
        <span className="text-xs font-semibold uppercase tracking-wide text-content-muted">
          {t('chat.files.panel.title').replace('{count}', String(artifacts.length))}
        </span>
        <Button
          iconOnly
          variant="tertiary"
          size="xs"
          onClick={onClose}
          data-analytics-id="chat-files-panel-close"
          aria-label={t('chat.files.panel.close')}>
          <svg
            className="w-3.5 h-3.5"
            fill="none"
            stroke="currentColor"
            strokeWidth={2}
            viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </Button>
      </header>

      {artifacts.length === 0 ? (
        <div className="px-3 py-6 text-xs text-content-muted text-center">
          {t('chat.files.panel.empty')}
        </div>
      ) : (
        <ul className="divide-y divide-line-subtle dark:divide-neutral-800">
          {artifacts.map(artifact => {
            const row = downloadState[artifact.artifactId] ?? { state: 'idle' as const };
            const isConfirming = confirmDeleteId === artifact.artifactId;
            return (
              <li
                key={artifact.artifactId}
                className="px-3 py-2.5 flex flex-col gap-1"
                data-testid={`chat-files-row-${artifact.artifactId}`}>
                <div className="flex items-center gap-2.5 text-sm text-content-secondary">
                  <KindIcon kind={artifact.kind} />
                  <div className="flex flex-col min-w-0 flex-1">
                    <span className="truncate font-medium leading-tight">{artifact.title}</span>
                    <span className="text-[11px] font-mono text-content-muted leading-tight">
                      {artifact.sizeBytes != null ? formatFileSize(artifact.sizeBytes) : ''}
                    </span>
                  </div>
                </div>
                {isConfirming ? (
                  <div className="flex items-center gap-2 mt-1">
                    <span className="text-[11px] text-content-secondary flex-1">
                      {t('chat.files.delete.confirm')}
                    </span>
                    <button
                      type="button"
                      onClick={() => setConfirmDeleteId(null)}
                      data-analytics-id="chat-files-delete-cancel"
                      className="rounded-md bg-surface-subtle hover:bg-surface-strong dark:hover:bg-neutral-700 text-content-secondary text-[11px] font-medium px-2 py-1 transition-colors">
                      {t('chat.files.delete.cancel')}
                    </button>
                    <button
                      type="button"
                      onClick={() => void handleDeleteConfirm(artifact)}
                      data-analytics-id={`chat-files-delete-confirm-${artifact.kind}`}
                      data-testid={`chat-files-confirm-${artifact.artifactId}`}
                      className="rounded-md bg-coral-500 hover:bg-coral-600 text-content-inverted text-[11px] font-medium px-2 py-1 transition-colors">
                      {t('chat.files.delete.action')}
                    </button>
                  </div>
                ) : (
                  <div className="flex items-center gap-2 mt-0.5">
                    <button
                      type="button"
                      onClick={() => void handleDownload(artifact)}
                      disabled={row.state === 'downloading'}
                      data-analytics-id={`chat-files-download-${artifact.kind}`}
                      data-testid={`chat-files-download-${artifact.artifactId}`}
                      className="rounded-md bg-ocean-500 hover:bg-ocean-600 disabled:bg-stone-300 dark:disabled:bg-neutral-700 text-white text-[11px] font-medium px-2 py-1 transition-colors">
                      {row.state === 'downloading'
                        ? t('chat.artifact.downloading')
                        : t('chat.artifact.download')}
                    </button>
                    {row.state === 'done' && row.path && (
                      <button
                        type="button"
                        onClick={() => void handleReveal(row.path!)}
                        data-analytics-id={`chat-files-reveal-${artifact.kind}`}
                        data-testid={`chat-files-reveal-${artifact.artifactId}`}
                        className="text-[11px] underline text-sage-700 dark:text-sage-300 hover:text-sage-900 dark:hover:text-sage-100">
                        {t('chat.artifact.reveal')}
                      </button>
                    )}
                    <button
                      type="button"
                      onClick={() => setConfirmDeleteId(artifact.artifactId)}
                      data-analytics-id={`chat-files-delete-${artifact.kind}`}
                      data-testid={`chat-files-delete-${artifact.artifactId}`}
                      aria-label={t('chat.files.delete.aria').replace('{title}', artifact.title)}
                      className="ml-auto rounded-md bg-transparent text-content-muted hover:bg-coral-50 dark:hover:bg-coral-900/20 hover:text-coral-700 dark:hover:text-coral-300 text-[11px] font-medium px-2 py-1 transition-colors">
                      <svg
                        className="w-3.5 h-3.5"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth={1.8}
                        viewBox="0 0 24 24">
                        <path
                          strokeLinecap="round"
                          strokeLinejoin="round"
                          d="M19 7l-1 12a2 2 0 01-2 2H8a2 2 0 01-2-2L5 7m5 4v6m4-6v6M4 7h16M9 7V4a1 1 0 011-1h4a1 1 0 011 1v3"
                        />
                      </svg>
                    </button>
                  </div>
                )}
                {row.state === 'error' && row.error && (
                  <p className="text-[11px] text-coral-600 dark:text-coral-400 mt-0.5 break-words">
                    {t('chat.artifact.download_failed').replace('{reason}', row.error)}
                  </p>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {deleteError && (
        <div className="px-3 py-2 border-t border-line-subtle text-[11px] text-coral-600 dark:text-coral-400 break-words">
          {deleteError}
        </div>
      )}
    </div>
  );
}
