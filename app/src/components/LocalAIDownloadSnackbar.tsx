import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

import {
  formatBytes,
  formatEta,
  progressFromDownloads,
  statusLabel,
} from '../utils/localAiHelpers';
import {
  isTauri,
  type LocalAiDownloadsProgress,
  type LocalAiStatus,
  openhumanLocalAiDownloadsProgress,
  openhumanLocalAiStatus,
} from '../utils/tauriCommands';

const POLL_INTERVAL = 2000;

/**
 * Persistent snackbar that shows local AI download progress.
 * Anchored bottom-right.
 * Dismiss hides the UI but does NOT cancel the download.
 */
const LocalAIDownloadSnackbar = () => {
  const [status, setStatus] = useState<LocalAiStatus | null>(null);
  const [downloads, setDownloads] = useState<LocalAiDownloadsProgress | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const timerRef = useRef<ReturnType<typeof setInterval>>(undefined);

  // Check Tauri availability once at init
  const tauriAvailable = (() => {
    try {
      return isTauri();
    } catch {
      return false;
    }
  })();

  // Poll download status
  useEffect(() => {
    if (!tauriAvailable) return;

    const poll = async () => {
      try {
        const [statusRes, downloadsRes] = await Promise.all([
          openhumanLocalAiStatus(),
          openhumanLocalAiDownloadsProgress(),
        ]);
        if (statusRes.result) setStatus(statusRes.result);
        if (downloadsRes.result) setDownloads(downloadsRes.result);
      } catch {
        // Silently ignore — core may not be ready
      }
    };

    void poll();
    timerRef.current = setInterval(poll, POLL_INTERVAL);
    return () => clearInterval(timerRef.current);
  }, [tauriAvailable]);

  const isDownloading =
    status?.state === 'downloading' ||
    status?.state === 'installing' ||
    downloads?.state === 'downloading' ||
    (downloads?.progress != null && downloads.progress > 0 && downloads.progress < 1);

  // Auto-show when a new download starts: track prior state in a ref and
  // reset dismissed on the transition edge (not-downloading → downloading).
  const wasDownloadingRef = useRef(false);
  if (isDownloading && !wasDownloadingRef.current && dismissed) {
    setDismissed(false);
  }
  wasDownloadingRef.current = !!isDownloading;

  const handleDismiss = useCallback(() => setDismissed(true), []);
  const handleToggleCollapse = useCallback(() => setCollapsed(prev => !prev), []);

  if (!tauriAvailable || !isDownloading || dismissed) return null;

  const progress = progressFromDownloads(downloads);
  const percent = progress != null ? Math.round(progress * 100) : null;
  const speed = downloads?.speed_bps;
  const eta = downloads?.eta_seconds;
  const downloaded = downloads?.downloaded_bytes;
  const total = downloads?.total_bytes;
  const currentState = downloads?.state ?? status?.state ?? 'downloading';
  const label = statusLabel(currentState);
  const isInstallingPhase = currentState === 'installing';
  const phaseDetail = downloads?.warning ?? status?.warning;

  // Collapsed: small pill
  if (collapsed) {
    return createPortal(
      <div className="fixed bottom-4 right-4 z-[9998] animate-fade-up">
        <button
          onClick={handleToggleCollapse}
          className="flex items-center gap-2 bg-stone-900 border border-stone-700/50 rounded-full px-3 py-2 shadow-large hover:border-stone-600 transition-colors"
          aria-label="Expand download progress">
          <svg
            className="w-4 h-4 text-primary-400 animate-pulse"
            viewBox="0 0 20 20"
            fill="currentColor">
            <path d="M10.75 2.75a.75.75 0 00-1.5 0v8.614L6.295 8.235a.75.75 0 10-1.09 1.03l4.25 4.5a.75.75 0 001.09 0l4.25-4.5a.75.75 0 00-1.09-1.03l-2.955 3.129V2.75z" />
            <path d="M3.5 12.75a.75.75 0 00-1.5 0v2.5A2.75 2.75 0 004.75 18h10.5A2.75 2.75 0 0018 15.25v-2.5a.75.75 0 00-1.5 0v2.5c0 .69-.56 1.25-1.25 1.25H4.75c-.69 0-1.25-.56-1.25-1.25v-2.5z" />
          </svg>
          <span className="text-xs font-medium text-stone-300">
            {percent != null ? `${percent}%` : label}
          </span>
        </button>
      </div>,
      document.body
    );
  }

  // Expanded: full snackbar
  return createPortal(
    <div className="fixed bottom-4 right-4 z-[9998] w-[320px] animate-fade-up">
      <div className="bg-stone-900 border border-stone-700/50 rounded-2xl shadow-large overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between px-4 pt-3 pb-1">
          <div className="flex items-center gap-2">
            <svg
              className="w-4 h-4 text-primary-400 animate-pulse"
              viewBox="0 0 20 20"
              fill="currentColor">
              <path d="M10.75 2.75a.75.75 0 00-1.5 0v8.614L6.295 8.235a.75.75 0 10-1.09 1.03l4.25 4.5a.75.75 0 001.09 0l4.25-4.5a.75.75 0 00-1.09-1.03l-2.955 3.129V2.75z" />
              <path d="M3.5 12.75a.75.75 0 00-1.5 0v2.5A2.75 2.75 0 004.75 18h10.5A2.75 2.75 0 0018 15.25v-2.5a.75.75 0 00-1.5 0v2.5c0 .69-.56 1.25-1.25 1.25H4.75c-.69 0-1.25-.56-1.25-1.25v-2.5z" />
            </svg>
            <span className="text-sm font-medium text-white">{label}</span>
          </div>
          <div className="flex items-center gap-1">
            <button
              onClick={handleToggleCollapse}
              className="p-1 text-stone-500 hover:text-stone-300 transition-colors"
              aria-label="Collapse download progress">
              <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="currentColor">
                <path d="M3.75 7.25a.75.75 0 000 1.5h8.5a.75.75 0 000-1.5h-8.5z" />
              </svg>
            </button>
            <button
              onClick={handleDismiss}
              className="p-1 text-stone-500 hover:text-stone-300 transition-colors"
              aria-label="Dismiss download notification">
              <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="currentColor">
                <path d="M4.28 3.22a.75.75 0 00-1.06 1.06L6.94 8l-3.72 3.72a.75.75 0 101.06 1.06L8 9.06l3.72 3.72a.75.75 0 101.06-1.06L9.06 8l3.72-3.72a.75.75 0 00-1.06-1.06L8 6.94 4.28 3.22z" />
              </svg>
            </button>
          </div>
        </div>

        {/* Phase detail */}
        {phaseDetail && (
          <div className="px-4 pb-1">
            <span className="text-[11px] text-stone-400 truncate block">{phaseDetail}</span>
          </div>
        )}

        {/* Progress bar */}
        <div className="px-4 py-2">
          <div className="h-1.5 w-full rounded-full bg-stone-800 overflow-hidden">
            <div
              className={`h-full rounded-full bg-gradient-to-r from-primary-500 to-primary-400 transition-all duration-500 ${
                isInstallingPhase ? 'animate-pulse' : ''
              }`}
              style={{
                width: isInstallingPhase ? '100%' : `${percent ?? 0}%`,
              }}
            />
          </div>
        </div>

        {/* Details */}
        <div className="flex items-center justify-between px-4 pb-3 text-xs text-stone-400">
          <span>
            {isInstallingPhase
              ? 'Installing...'
              : downloaded != null && total != null
                ? `${formatBytes(downloaded)} / ${formatBytes(total)}`
                : percent != null
                  ? `${percent}%`
                  : 'Preparing...'}
          </span>
          <span>
            {speed != null && speed > 0 ? `${formatBytes(speed)}/s` : ''}
            {eta != null && eta > 0 ? ` · ${formatEta(eta)}` : ''}
          </span>
        </div>
      </div>
    </div>,
    document.body
  );
};

export default LocalAIDownloadSnackbar;
