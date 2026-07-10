import { useState } from 'react';

import { formatFileSize } from '../../lib/attachments';
import { useT } from '../../lib/i18n/I18nContext';
import {
  revealArtifactInFileManager,
  saveArtifactViaDialog,
} from '../../services/artifactDownloadService';
import type { ArtifactSnapshot } from '../../store/chatRuntimeSlice';

/**
 * Inline chat card surfacing a single agent-generated artifact (#2779).
 *
 * Renders three visual states keyed off `artifact.status`:
 *
 * - `in_progress` — pulsing dot + title + "Generating <kind>…" label.
 *   Derived state: a `ChatToolCallEvent` for an artifact-producing
 *   tool was seen but no `artifact_ready` / `artifact_failed` has
 *   landed yet.
 * - `ready` — kind icon + title + human-readable size + Download
 *   button. Click → `downloadArtifact()` → "Saved to …" w/ a
 *   "Show in folder" link.
 * - `failed` — error icon + title + producer-supplied reason +
 *   optional Retry button (only when `onRetry` is provided).
 *
 * Visual style mirrors `ApprovalRequestCard` / `AttachmentPreview`:
 * rounded card, dark/light Tailwind variants, mono accents on
 * numeric values, inline SVG icons. No new icon dependency.
 */
export interface ArtifactCardProps {
  artifact: ArtifactSnapshot;
  /** When provided, render a Retry button on the `failed` state. */
  onRetry?: (artifactId: string) => void;
}

/**
 * Extension hint for the Tauri download command. Falls back to a
 * lowercased kind slug when the title doesn't carry an explicit
 * extension (defensive — `create_artifact` sanitises the title +
 * extension separately, but a malformed title shouldn't crash the
 * card).
 */
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
          className="w-5 h-5 flex-shrink-0"
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
          className="w-5 h-5 flex-shrink-0"
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
          className="w-5 h-5 flex-shrink-0"
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
          className="w-5 h-5 flex-shrink-0"
          fill="none"
          stroke={stroke}
          strokeWidth={1.8}
          viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" d="M4 4h16v16H4z" />
        </svg>
      );
  }
}

function Spinner() {
  return (
    <svg
      aria-hidden="true"
      className="w-5 h-5 flex-shrink-0 animate-spin"
      fill="none"
      viewBox="0 0 24 24">
      <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="3" />
      <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v3a5 5 0 00-5 5H4z" />
    </svg>
  );
}

function FailedIcon() {
  return (
    <svg
      aria-hidden="true"
      className="w-5 h-5 flex-shrink-0 text-coral-500"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      viewBox="0 0 24 24">
      <circle cx="12" cy="12" r="9" />
      <path strokeLinecap="round" d="M12 8v4M12 16h.01" />
    </svg>
  );
}

/**
 * Cap the visible failure reason at ~280 chars. Producer-side errors
 * can be enormous (e.g. a multi-KB pip stderr from a failed venv
 * setup — observed at 13K chars in dev:app on 2026-05-30) and
 * dumping that raw into a flex card breaks layout + can freeze the
 * scrolling page. We collapse by default and let the user expand
 * via "Show more" if they actually want to read it.
 */
const ERROR_REASON_PREVIEW_CHARS = 280;

export default function ArtifactCard({ artifact, onRetry }: ArtifactCardProps) {
  const { t } = useT();
  const [download, setDownload] = useState<{
    state: 'idle' | 'downloading' | 'done' | 'error';
    path?: string;
    error?: string;
  }>({ state: 'idle' });
  const [errorExpanded, setErrorExpanded] = useState(false);

  const handleDownload = async () => {
    setDownload({ state: 'downloading' });
    const ext = extensionFor(artifact.kind, artifact.title);
    const outcome = await saveArtifactViaDialog(artifact.artifactId, artifact.title, ext);
    if (outcome.ok) {
      setDownload({ state: 'done', path: outcome.path });
    } else if (outcome.code === 'CANCELLED') {
      // User dismissed the Save-As dialog — quietly return to idle.
      setDownload({ state: 'idle' });
    } else {
      setDownload({ state: 'error', error: outcome.error });
    }
  };

  const handleReveal = async () => {
    if (download.path) {
      await revealArtifactInFileManager(download.path);
    }
  };

  return (
    <div
      role="group"
      aria-label={t('chat.artifact.aria').replace('{title}', artifact.title)}
      className="flex flex-col gap-1.5 rounded-xl border border-line bg-surface-muted px-3 py-2.5 text-sm text-content-secondary max-w-[420px]">
      <div className="flex items-center gap-2.5">
        {artifact.status === 'in_progress' ? (
          <Spinner />
        ) : artifact.status === 'failed' ? (
          <FailedIcon />
        ) : (
          <KindIcon kind={artifact.kind} />
        )}
        <div className="flex flex-col min-w-0 flex-1">
          <span className="truncate font-medium leading-tight">{artifact.title}</span>
          <span className="text-xs text-content-muted leading-tight font-mono">
            {artifact.status === 'in_progress'
              ? t('chat.artifact.generating').replace('{kind}', artifact.kind)
              : artifact.status === 'ready' && artifact.sizeBytes != null
                ? `${t('chat.artifact.ready')} · ${formatFileSize(artifact.sizeBytes)}`
                : artifact.status === 'failed'
                  ? t('chat.artifact.failed')
                  : ''}
          </span>
        </div>
        {artifact.status === 'ready' && download.state !== 'done' && (
          <button
            type="button"
            data-analytics-id={`chat-artifact-download-${artifact.kind}`}
            onClick={handleDownload}
            disabled={download.state === 'downloading'}
            className="ml-auto rounded-md bg-ocean-500 hover:bg-ocean-600 disabled:bg-stone-300 dark:disabled:bg-neutral-700 text-white text-xs font-medium px-3 py-1.5 transition-colors">
            {download.state === 'downloading'
              ? t('chat.artifact.downloading')
              : t('chat.artifact.download')}
          </button>
        )}
        {artifact.status === 'failed' && onRetry && (
          <button
            type="button"
            data-analytics-id={`chat-artifact-retry-${artifact.kind}`}
            onClick={() => onRetry(artifact.artifactId)}
            className="ml-auto rounded-md bg-surface-strong hover:bg-stone-300 dark:hover:bg-neutral-600 text-content-secondary text-xs font-medium px-3 py-1.5 transition-colors">
            {t('chat.artifact.retry')}
          </button>
        )}
      </div>
      {artifact.status === 'failed' && artifact.error && (
        <div className="text-xs text-coral-600 dark:text-coral-400 mt-1">
          <p
            className={`font-mono break-words whitespace-pre-wrap ${
              errorExpanded ? 'max-h-48 overflow-y-auto' : ''
            }`}>
            {errorExpanded || artifact.error.length <= ERROR_REASON_PREVIEW_CHARS
              ? artifact.error
              : `${artifact.error.slice(0, ERROR_REASON_PREVIEW_CHARS)}…`}
          </p>
          {artifact.error.length > ERROR_REASON_PREVIEW_CHARS && (
            <button
              type="button"
              data-analytics-id="chat-artifact-error-toggle"
              onClick={() => setErrorExpanded(prev => !prev)}
              className="mt-1 underline text-coral-700 dark:text-coral-300 hover:text-coral-900 dark:hover:text-coral-100">
              {errorExpanded ? t('chat.artifact.show_less') : t('chat.artifact.show_more')}
            </button>
          )}
        </div>
      )}
      {download.state === 'done' && download.path && (
        <div className="flex items-center gap-2 text-xs text-sage-700 dark:text-sage-300 mt-1">
          <span className="truncate font-mono">
            {t('chat.artifact.downloaded').replace('{path}', download.path)}
          </span>
          <button
            type="button"
            data-analytics-id={`chat-artifact-reveal-${artifact.kind}`}
            onClick={handleReveal}
            className="ml-auto underline hover:text-sage-900 dark:hover:text-sage-100 transition-colors flex-shrink-0">
            {t('chat.artifact.reveal')}
          </button>
        </div>
      )}
      {download.state === 'error' && download.error && (
        <p className="text-xs text-coral-600 dark:text-coral-400 mt-1 break-words">
          {t('chat.artifact.download_failed').replace('{reason}', download.error)}
        </p>
      )}
    </div>
  );
}
