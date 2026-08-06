import debugFactory from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

import { useT } from '../lib/i18n/I18nContext';
import { useCoreState } from '../providers/CoreStateProvider';
import {
  formatBytes,
  formatEta,
  progressFromDownloads,
  progressFromStatus,
  statusLabel,
} from '../utils/localAiHelpers';
import {
  isTauri,
  type LocalAiDownloadsProgress,
  type LocalAiStatus,
  openhumanLocalAiDownloadsProgress,
  openhumanLocalAiStatus,
} from '../utils/tauriCommands';
import Button from './ui/Button';

const log = debugFactory('local-ai-download');

const ACTIVE_POLL_INTERVAL = 2000;

const IN_FLIGHT_STATES = new Set(['loading', 'downloading', 'installing']);

/** Whether a `LocalAiStatus.state` string denotes an active download/bootstrap. */
const isInFlightState = (state: string | undefined): boolean =>
  state != null && IN_FLIGHT_STATES.has(state);

/**
 * Pure predicate deciding whether a download/bootstrap is currently in flight,
 * mirroring the `isDownloading` derivation used for rendering. Drives the fast
 * poll's continuation: keep polling `inference_downloads_progress` +
 * `inference_status` only while a download is genuinely in flight.
 */
const isDownloadInFlight = (
  status: LocalAiStatus | null,
  downloads: LocalAiDownloadsProgress | null
): boolean => {
  const downloadState = downloads?.state;
  const currentState = isInFlightState(downloadState)
    ? downloadState
    : (status?.state ?? downloadState ?? 'idle');
  return (
    isInFlightState(currentState) ||
    (downloads?.progress != null && downloads.progress > 0 && downloads.progress < 1)
  );
};

/**
 * Persistent snackbar that shows local AI download progress.
 * Anchored bottom-right.
 * Dismiss hides the UI but does NOT cancel the download.
 */
const LocalAIDownloadSnackbar = () => {
  const { t } = useT();
  const [status, setStatus] = useState<LocalAiStatus | null>(null);
  const [downloads, setDownloads] = useState<LocalAiDownloadsProgress | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);
  // Track previous isDownloading in state so we can reset the dismiss flag on a
  // not-downloading → downloading transition during render (render-phase update,
  // the officially recommended React pattern for adjusting state on derived-value changes).
  const [prevIsDownloading, setPrevIsDownloading] = useState(false);

  // Check Tauri availability once at init
  const tauriAvailable = (() => {
    try {
      return isTauri();
    } catch {
      return false;
    }
  })();

  // Detect an active download from the folded local-AI state in the app-state
  // snapshot (polled by CoreStateProvider) instead of a dedicated idle poll of
  // the inference RPCs. When idle this component issues ZERO inference calls;
  // the snapshot's `runtime.localAi.state` is what flips us into the fast poll.
  const { snapshot: coreSnapshot } = useCoreState();
  const coreDownloadActive = isInFlightState(coreSnapshot.runtime.localAi?.state ?? undefined);

  // While a download is in flight, poll the inference RPCs fast for smooth,
  // granular progress/speed/ETA (the 2–5s app-state cadence is too coarse and
  // carries no downloads-progress detail). The poll starts when core state
  // reports activity and keeps going as long as the download itself is in
  // flight, then stops — so there is no steady-state inference polling.
  useEffect(() => {
    if (!tauriAvailable || !coreDownloadActive) return;

    let cancelled = false;
    log('fast poll: starting (core reports download active)');

    const poll = async () => {
      let settled = false;
      try {
        const [statusRes, downloadsRes] = await Promise.all([
          openhumanLocalAiStatus(),
          openhumanLocalAiDownloadsProgress(),
        ]);
        if (statusRes.result) setStatus(statusRes.result);
        if (downloadsRes.result) setDownloads(downloadsRes.result);
        // The download reached a terminal state — stop the fast poll early
        // rather than waiting for core state to catch up (it lags up to ~5s).
        // A successful response that carries no `result` (a soft failure, not a
        // thrown error) must be treated as transient, not as "download
        // complete" — otherwise one empty blip would permanently freeze the
        // fast poll for the rest of the download (same failure mode the catch
        // below guards against for thrown errors).
        if (statusRes.result || downloadsRes.result) {
          settled = !isDownloadInFlight(statusRes.result ?? null, downloadsRes.result ?? null);
        }
      } catch (err) {
        // Transient RPC failure (core may be busy). Do NOT treat this as
        // settled: keep polling while core state still reports the download
        // active, otherwise one blip would permanently stop progress updates
        // for the rest of the download (the effect won't re-run until
        // `coreDownloadActive` flips).
        log('fast poll: transient error, will retry: %O', err);
      }
      if (!cancelled && !settled) {
        timerRef.current = setTimeout(poll, ACTIVE_POLL_INTERVAL);
      } else if (settled) {
        log('fast poll: download settled, stopping');
      }
    };

    void poll();
    return () => {
      cancelled = true;
      if (timerRef.current) clearTimeout(timerRef.current);
      log('fast poll: stopped (unmount or core reports inactive)');
    };
  }, [tauriAvailable, coreDownloadActive]);

  const downloadState = downloads?.state;
  const currentState =
    downloadState === 'loading' || downloadState === 'downloading' || downloadState === 'installing'
      ? downloadState
      : (status?.state ?? downloadState ?? 'idle');
  // Gate on `coreDownloadActive` so a download that the core no longer reports
  // active can't leave the snackbar stuck: when the folded state flips inactive,
  // polling stops and any still-"downloading" local status/downloads must not
  // keep the overlay visible.
  const isDownloading =
    coreDownloadActive &&
    (currentState === 'loading' ||
      currentState === 'downloading' ||
      currentState === 'installing' ||
      (downloads?.progress != null && downloads.progress > 0 && downloads.progress < 1));

  // Render-phase update: when a new download cycle starts (not-downloading → downloading),
  // reset the dismiss/collapsed flags so the snackbar reappears automatically.
  if (!!isDownloading !== prevIsDownloading) {
    setPrevIsDownloading(!!isDownloading);
    if (isDownloading && !prevIsDownloading) {
      setDismissed(false);
      setCollapsed(false);
    }
  }

  const handleDismiss = useCallback(() => setDismissed(true), []);
  const handleToggleCollapse = useCallback(() => setCollapsed(prev => !prev), []);

  if (!tauriAvailable || !isDownloading || dismissed) return null;

  // Use currentState as the source of truth for the fallback sentinel so the
  // label (derived from currentState) and the progress bar stay in sync.
  // We still forward download_progress from status so a real numeric value
  // isn't lost when the downloads object has no progress field.
  // When status is absent, progressFromStatus(null) returns 0, which is the
  // correct baseline while data hasn't arrived yet.
  const statusForProgress: LocalAiStatus | null = status
    ? { ...status, state: currentState }
    : null;
  const progress = progressFromDownloads(downloads) ?? progressFromStatus(statusForProgress);
  const percent = progress != null ? Math.round(progress * 100) : null;
  const speed = downloads?.speed_bps ?? status?.download_speed_bps;
  const eta = downloads?.eta_seconds ?? status?.eta_seconds;
  const downloaded = downloads?.downloaded_bytes ?? status?.downloaded_bytes;
  const total = downloads?.total_bytes ?? status?.total_bytes;
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
          aria-label={t('app.localAiDownload.expandAria')}>
          <svg
            className="w-4 h-4 text-primary-400 animate-pulse"
            viewBox="0 0 20 20"
            fill="currentColor">
            <path d="M10.75 2.75a.75.75 0 00-1.5 0v8.614L6.295 8.235a.75.75 0 10-1.09 1.03l4.25 4.5a.75.75 0 001.09 0l4.25-4.5a.75.75 0 00-1.09-1.03l-2.955 3.129V2.75z" />
            <path d="M3.5 12.75a.75.75 0 00-1.5 0v2.5A2.75 2.75 0 004.75 18h10.5A2.75 2.75 0 0018 15.25v-2.5a.75.75 0 00-1.5 0v2.5c0 .69-.56 1.25-1.25 1.25H4.75c-.69 0-1.25-.56-1.25-1.25v-2.5z" />
          </svg>
          <span className="text-xs font-medium text-content-faint">
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
            <Button
              iconOnly
              variant="tertiary"
              size="xs"
              onClick={handleToggleCollapse}
              aria-label={t('app.localAiDownload.collapseAria')}>
              <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="currentColor">
                <path d="M3.75 7.25a.75.75 0 000 1.5h8.5a.75.75 0 000-1.5h-8.5z" />
              </svg>
            </Button>
            <Button
              iconOnly
              variant="tertiary"
              size="xs"
              onClick={handleDismiss}
              aria-label={t('app.localAiDownload.dismissAria')}>
              <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="currentColor">
                <path d="M4.28 3.22a.75.75 0 00-1.06 1.06L6.94 8l-3.72 3.72a.75.75 0 101.06 1.06L8 9.06l3.72 3.72a.75.75 0 101.06-1.06L9.06 8l3.72-3.72a.75.75 0 00-1.06-1.06L8 6.94 4.28 3.22z" />
              </svg>
            </Button>
          </div>
        </div>

        {/* Phase detail */}
        {phaseDetail && (
          <div className="px-4 pb-1">
            <span className="text-[11px] text-content-faint truncate block">{phaseDetail}</span>
          </div>
        )}

        {/* Progress bar */}
        <div className="px-4 py-2">
          <div className="h-1.5 w-full rounded-full bg-stone-800 overflow-hidden">
            <div
              className={`h-full rounded-full bg-gradient-to-r from-primary-500 to-primary-400 transition-all duration-500 ${
                isInstallingPhase ? 'animate-pulse' : ''
              }`}
              style={{ width: isInstallingPhase ? '100%' : `${percent ?? 0}%` }}
            />
          </div>
        </div>

        {/* Details */}
        <div className="flex items-center justify-between px-4 pb-3 text-xs text-content-faint">
          <span>
            {isInstallingPhase
              ? t('app.localAiDownload.installing')
              : downloaded != null && total != null
                ? `${formatBytes(downloaded)} / ${formatBytes(total)}`
                : percent != null
                  ? `${percent}%`
                  : t('app.localAiDownload.preparing')}
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
