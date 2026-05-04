/**
 * App auto-update prompt.
 *
 * Globally-mounted banner that surfaces the Tauri shell updater to the user.
 * The state machine, listeners, and auto-download orchestration all live in
 * `useAppUpdate`; this component is a thin presentational layer on top.
 *
 * UX contract: the banner is **silent during background download**. The user
 * only sees a prompt once bytes are staged (`ready_to_install`) — at which
 * point they can choose "Restart now" or "Later". Errors and the active
 * install/restart flow also surface visually.
 *
 * Visual conventions mirror `LocalAIDownloadSnackbar` — bottom-right portal,
 * stone-900 panel, primary gradient progress bar.
 */
import { useCallback, useState } from 'react';
import { createPortal } from 'react-dom';

import { useAppUpdate } from '../hooks/useAppUpdate';
import { formatBytes } from '../utils/localAiHelpers';

interface AppUpdatePromptProps {
  /** Override auto-check defaults (mostly for tests). */
  autoCheck?: boolean;
  initialCheckDelayMs?: number;
  recheckIntervalMs?: number;
  autoDownload?: boolean;
}

/**
 * Phases that should surface a visible banner. Background-only phases
 * (`checking`, `available`, `downloading`) stay silent so the user isn't
 * pestered while we're working — the prompt only appears once the user
 * has a meaningful decision to make.
 */
function shouldShow(phase: ReturnType<typeof useAppUpdate>['phase']): boolean {
  return (
    phase === 'ready_to_install' ||
    phase === 'installing' ||
    phase === 'restarting' ||
    phase === 'error'
  );
}

const AppUpdatePrompt = (props: AppUpdatePromptProps) => {
  const { phase, info, bytesDownloaded, totalBytes, error, install, download, reset } =
    useAppUpdate(props);

  const [dismissed, setDismissed] = useState(false);
  const [prevPhase, setPrevPhase] = useState(phase);
  // Re-show on every transition INTO a visible phase, even if the user had
  // dismissed a previous error/prompt earlier in the session.
  if (phase !== prevPhase) {
    setPrevPhase(phase);
    if (shouldShow(phase) && !shouldShow(prevPhase)) {
      setDismissed(false);
    }
  }

  const handleInstall = useCallback(() => {
    void install();
  }, [install]);

  const handleLater = useCallback(() => {
    setDismissed(true);
  }, []);

  const handleRetryDownload = useCallback(() => {
    setDismissed(false);
    reset();
    void download();
  }, [reset, download]);

  const handleDismissError = useCallback(() => {
    reset();
    setDismissed(true);
  }, [reset]);

  if (!shouldShow(phase) || dismissed) return null;

  const newVersion = info?.available_version ?? null;
  const currentVersion = info?.current_version ?? null;
  const percent =
    totalBytes != null && totalBytes > 0
      ? Math.min(100, Math.round((bytesDownloaded / totalBytes) * 100))
      : null;

  return createPortal(
    <div
      role="status"
      aria-live="polite"
      className="fixed bottom-4 right-4 z-[9998] w-[340px] animate-fade-up"
      data-testid="app-update-prompt">
      <div className="bg-stone-900 border border-stone-700/50 rounded-2xl shadow-large overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between px-4 pt-3 pb-1">
          <div className="flex items-center gap-2">
            <UpdateIcon className="w-4 h-4 text-primary-400" />
            <span className="text-sm font-medium text-white">{headerLabel(phase)}</span>
          </div>
          {(phase === 'ready_to_install' || phase === 'error') && (
            <button
              onClick={phase === 'error' ? handleDismissError : handleLater}
              className="p-1 text-stone-500 hover:text-stone-300 transition-colors"
              aria-label="Dismiss update notification">
              <CloseIcon className="w-3.5 h-3.5" />
            </button>
          )}
        </div>

        {/* Body */}
        <div className="px-4 pt-1 pb-3">
          {phase === 'ready_to_install' && (
            <>
              <p className="text-xs text-stone-300 leading-relaxed">
                {newVersion
                  ? `Version ${newVersion} is ready to install.`
                  : 'A new version is ready to install.'}
                {currentVersion && (
                  <span className="text-stone-500"> Currently on {currentVersion}.</span>
                )}
              </p>
              {info?.body && <ReleaseNotes body={info.body} />}
              <p className="mt-2 text-[11px] text-stone-500 leading-relaxed">
                Restarting will close any open conversations briefly. The new build launches
                automatically.
              </p>
              <div className="mt-3 flex gap-2">
                <button
                  onClick={handleInstall}
                  className="flex-1 px-3 py-1.5 rounded-lg bg-primary-500 hover:bg-primary-400 text-white text-xs font-medium transition-colors">
                  Restart now
                </button>
                <button
                  onClick={handleLater}
                  className="px-3 py-1.5 rounded-lg border border-stone-700 text-stone-300 hover:bg-stone-800 text-xs transition-colors">
                  Later
                </button>
              </div>
            </>
          )}

          {(phase === 'installing' || phase === 'restarting') && (
            <>
              <ProgressBar indeterminate />
              <div className="mt-2 flex items-center justify-between text-[11px] text-stone-400">
                <span>{progressDetail(phase, bytesDownloaded, totalBytes, percent)}</span>
                {newVersion && <span className="text-stone-500">v{newVersion}</span>}
              </div>
            </>
          )}

          {phase === 'error' && (
            <>
              <p className="text-xs text-coral-300 leading-relaxed">
                {error ?? 'Something went wrong while updating.'}
              </p>
              <div className="mt-3 flex gap-2">
                <button
                  onClick={handleRetryDownload}
                  className="flex-1 px-3 py-1.5 rounded-lg bg-primary-500 hover:bg-primary-400 text-white text-xs font-medium transition-colors">
                  Try again
                </button>
                <button
                  onClick={handleDismissError}
                  className="px-3 py-1.5 rounded-lg border border-stone-700 text-stone-300 hover:bg-stone-800 text-xs transition-colors">
                  Dismiss
                </button>
              </div>
            </>
          )}
        </div>
      </div>
    </div>,
    document.body
  );
};

function headerLabel(phase: ReturnType<typeof useAppUpdate>['phase']): string {
  switch (phase) {
    case 'ready_to_install':
      return 'Update ready to install';
    case 'installing':
      return 'Installing update';
    case 'restarting':
      return 'Restarting…';
    case 'error':
      return 'Update failed';
    default:
      return 'Update';
  }
}

function progressDetail(
  phase: ReturnType<typeof useAppUpdate>['phase'],
  downloaded: number,
  total: number | null,
  percent: number | null
): string {
  if (phase === 'installing') return 'Installing the new version…';
  if (phase === 'restarting') return 'Relaunching the app…';
  if (total != null && total > 0) {
    return `${formatBytes(downloaded)} / ${formatBytes(total)}`;
  }
  if (downloaded > 0) return `${formatBytes(downloaded)} downloaded`;
  return percent != null ? `${percent}%` : 'Working…';
}

const ProgressBar = ({
  indeterminate,
  percent,
}: {
  indeterminate?: boolean;
  percent?: number | null;
}) => {
  const indet = indeterminate || percent == null;
  return (
    <div className="h-1.5 w-full rounded-full bg-stone-800 overflow-hidden">
      <div
        className={`h-full rounded-full bg-gradient-to-r from-primary-500 to-primary-400 transition-all duration-500 ${
          indet ? 'animate-pulse' : ''
        }`}
        style={{ width: indet ? '100%' : `${percent ?? 0}%` }}
      />
    </div>
  );
};

const ReleaseNotes = ({ body }: { body: string }) => {
  const [expanded, setExpanded] = useState(false);
  const trimmed = body.trim();
  if (!trimmed) return null;
  const isLong = trimmed.length > 160;
  const display = expanded || !isLong ? trimmed : `${trimmed.slice(0, 160).trimEnd()}…`;
  return (
    <div className="mt-2 rounded-lg bg-stone-800/60 border border-stone-700/40 px-3 py-2">
      <p className="text-[11px] text-stone-400 whitespace-pre-line break-words">{display}</p>
      {isLong && (
        <button
          type="button"
          onClick={() => setExpanded(prev => !prev)}
          className="mt-1 text-[11px] text-primary-300 hover:text-primary-200 transition-colors">
          {expanded ? 'Show less' : 'Show more'}
        </button>
      )}
    </div>
  );
};

const UpdateIcon = ({ className }: { className?: string }) => (
  <svg className={className} viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
    <path d="M10 2a8 8 0 015.292 13.97v1.78a.75.75 0 01-1.5 0v-1.06a.75.75 0 01.22-.53A6.5 6.5 0 1010 16.5a.75.75 0 010 1.5A8 8 0 1110 2z" />
    <path d="M9.25 6.75a.75.75 0 011.5 0v3.69l2.22 2.22a.75.75 0 11-1.06 1.06l-2.44-2.44a.75.75 0 01-.22-.53V6.75z" />
  </svg>
);

const CloseIcon = ({ className }: { className?: string }) => (
  <svg className={className} viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
    <path d="M4.28 3.22a.75.75 0 00-1.06 1.06L6.94 8l-3.72 3.72a.75.75 0 101.06 1.06L8 9.06l3.72 3.72a.75.75 0 101.06-1.06L9.06 8l3.72-3.72a.75.75 0 00-1.06-1.06L8 6.94 4.28 3.22z" />
  </svg>
);

export default AppUpdatePrompt;
