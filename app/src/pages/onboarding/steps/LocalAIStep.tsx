import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import {
  type LocalAiAssetsStatus,
  type LocalAiDownloadsProgress,
  type LocalAiStatus,
  openhumanLocalAiAssetsStatus,
  openhumanLocalAiDownload,
  openhumanLocalAiDownloadAllAssets,
  openhumanLocalAiDownloadsProgress,
  openhumanLocalAiStatus,
} from '../../../utils/tauriCommands';

/* ---------- helpers (from LocalModelPanel) ---------- */

const formatBytes = (bytes?: number | null): string => {
  if (typeof bytes !== 'number' || !Number.isFinite(bytes) || bytes < 0) return '0 B';
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = bytes / 1024;
  let unit = units[0];
  for (let i = 1; i < units.length && value >= 1024; i += 1) {
    value /= 1024;
    unit = units[i];
  }
  return `${value.toFixed(value >= 10 ? 0 : 1)} ${unit}`;
};

const formatEta = (etaSeconds?: number | null): string => {
  if (typeof etaSeconds !== 'number' || !Number.isFinite(etaSeconds) || etaSeconds <= 0) return '';
  const mins = Math.floor(etaSeconds / 60);
  const secs = etaSeconds % 60;
  if (mins <= 0) return `${secs}s`;
  return `${mins}m ${secs.toString().padStart(2, '0')}s`;
};

const progressFromDownloads = (downloads: LocalAiDownloadsProgress | null): number | null => {
  if (!downloads) return null;
  if (typeof downloads.progress !== 'number') return null;
  return Math.max(0, Math.min(1, downloads.progress));
};

const progressFromStatus = (status: LocalAiStatus | null): number => {
  if (!status) return 0;
  if (typeof status.download_progress === 'number')
    return Math.max(0, Math.min(1, status.download_progress));
  switch (status.state) {
    case 'ready':
      return 1;
    case 'loading':
      return 0.92;
    case 'downloading':
      return 0.25;
    default:
      return 0;
  }
};

const statusLabel = (state: string): string => {
  switch (state) {
    case 'ready':
      return 'Ready';
    case 'downloading':
      return 'Downloading';
    case 'loading':
      return 'Loading model...';
    case 'degraded':
      return 'Needs Attention';
    case 'disabled':
      return 'Disabled';
    case 'idle':
      return 'Idle';
    default:
      return state;
  }
};

/* ---------- component ---------- */

interface LocalAIStepProps {
  onNext: (result: { consentGiven: boolean; downloadStarted: boolean }) => void;
}

const LocalAIStep = ({ onNext }: LocalAIStepProps) => {
  const [consent, setConsent] = useState<boolean | null>(null);
  const [downloadStarted, setDownloadStarted] = useState(false);

  const [status, setStatus] = useState<LocalAiStatus | null>(null);
  const [_assets, setAssets] = useState<LocalAiAssetsStatus | null>(null);
  const [downloads, setDownloads] = useState<LocalAiDownloadsProgress | null>(null);
  const [error, setError] = useState('');
  const [isTriggeringDownload, setIsTriggeringDownload] = useState(false);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const progress = useMemo(() => {
    const dp = progressFromDownloads(downloads);
    if (dp != null) return dp;
    return progressFromStatus(status);
  }, [downloads, status]);

  const isIndeterminate =
    (downloads?.state ?? status?.state) === 'downloading' &&
    typeof downloads?.progress !== 'number' &&
    typeof status?.download_progress !== 'number';

  const downloadedBytes = downloads?.downloaded_bytes ?? status?.downloaded_bytes;
  const totalBytes = downloads?.total_bytes ?? status?.total_bytes;
  const speedBps = downloads?.speed_bps ?? status?.download_speed_bps;
  const etaSeconds = downloads?.eta_seconds ?? status?.eta_seconds;

  const isReady = status?.state === 'ready';

  const loadStatus = useCallback(async () => {
    try {
      const [statusRes, assetsRes, downloadsRes] = await Promise.all([
        openhumanLocalAiStatus(),
        openhumanLocalAiAssetsStatus(),
        openhumanLocalAiDownloadsProgress(),
      ]);
      setStatus(statusRes.result);
      setAssets(assetsRes.result);
      setDownloads(downloadsRes.result);
      setError('');
    } catch {
      /* status polling is best-effort */
    }
  }, []);

  const startPolling = useCallback(() => {
    if (pollRef.current) return;
    void loadStatus();
    pollRef.current = setInterval(() => void loadStatus(), 1500);
  }, [loadStatus]);

  const stopPolling = useCallback(() => {
    if (pollRef.current) {
      clearInterval(pollRef.current);
      pollRef.current = null;
    }
  }, []);

  useEffect(() => () => stopPolling(), [stopPolling]);

  // Auto-trigger download when user gives consent
  useEffect(() => {
    if (consent !== true || downloadStarted) return;

    const trigger = async () => {
      setIsTriggeringDownload(true);
      setError('');
      try {
        await openhumanLocalAiDownload(false);
        await openhumanLocalAiDownloadAllAssets(false);
        setDownloadStarted(true);
        startPolling();
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to start download');
      } finally {
        setIsTriggeringDownload(false);
      }
    };
    void trigger();
  }, [consent, downloadStarted, startPolling]);

  /* ---------- Phase 1: consent ---------- */
  if (consent === null) {
    return (
      <div className="rounded-3xl border border-stone-700 bg-stone-900 p-8 shadow-large animate-fade-up">
        <div className="text-center mb-5">
          <h1 className="text-xl font-bold mb-2">Download Local AI Models</h1>
          <p className="opacity-70 text-sm">
            OpenHuman uses local AI models directly on your device for faster, more private
            assistance. You can always change this later in Settings.
          </p>
        </div>

        <div className="space-y-3 mb-5">
          <div className="rounded-2xl border border-sage-500/30 bg-sage-500/10 p-3">
            <p className="text-sm font-medium mb-1">Complete Privacy</p>
            <p className="text-xs opacity-80">
              All your private & sensitive data gets processed locally by your local AI model. No
              data is sent to any third party.
            </p>
          </div>
          <div className="rounded-2xl border border-sage-500/30 bg-sage-500/10 p-3">
            <p className="text-sm font-medium mb-1">Absolutely Free</p>
            <p className="text-xs opacity-80">
              Running local AI models is free and does not require any subscription or payment.
            </p>
          </div>
          <div className="rounded-2xl border border-amber-500/30 bg-amber-500/10 p-3">
            <p className="text-sm font-medium mb-1">Resource impact</p>
            <p className="text-xs opacity-80">
              Typical setup needs 1-3 GB disk for model files and can use 1-2 GB RAM while running.
            </p>
          </div>
        </div>

        <div className="grid grid-cols-2 gap-2 mb-4">
          <button
            onClick={() => setConsent(false)}
            className="py-2.5 text-sm font-medium rounded-xl border transition-colors border-stone-600 hover:border-stone-500">
            Skip
          </button>
          <button
            onClick={() => setConsent(true)}
            className="py-2.5 btn-primary text-sm font-medium rounded-xl border transition-colors border-stone-600 hover:border-sage-500 hover:bg-sage-500/10">
            Download Local Models
          </button>
        </div>
      </div>
    );
  }

  /* ---------- Phase 2: consent=false, skip ---------- */
  if (consent === false) {
    return (
      <div className="rounded-3xl border border-stone-700 bg-stone-900 p-8 shadow-large animate-fade-up">
        <div className="text-center mb-5">
          <h1 className="text-xl font-bold mb-2">Local AI Models</h1>
          <p className="opacity-70 text-sm">
            No worries — you can always enable local models later in Settings.
          </p>
        </div>
        <button
          onClick={() => onNext({ consentGiven: false, downloadStarted: false })}
          className="btn-primary w-full py-2.5 text-sm font-medium rounded-xl">
          Continue
        </button>
      </div>
    );
  }

  /* ---------- Phase 3: consent=true, downloading ---------- */
  const downloadedText =
    typeof downloadedBytes === 'number'
      ? `${formatBytes(downloadedBytes)}${typeof totalBytes === 'number' ? ` / ${formatBytes(totalBytes)}` : ''}`
      : '';
  const speedText =
    typeof speedBps === 'number' && speedBps > 0 ? `${formatBytes(speedBps)}/s` : '';
  const etaText = formatEta(etaSeconds);

  return (
    <div className="glass rounded-3xl p-8 shadow-large animate-fade-up">
      <div className="text-center mb-5">
        <h1 className="text-xl font-bold mb-2">Downloading Local Models</h1>
        <p className="opacity-70 text-sm">
          {isReady
            ? 'Models are ready! You can continue to the next step.'
            : 'Download is running in the background. You can continue while it finishes.'}
        </p>
      </div>

      {/* Progress bar */}
      <div className="mb-4">
        <div className="flex items-center justify-between text-xs opacity-70 mb-1.5">
          <span>{status?.state ? statusLabel(status.state) : 'Preparing...'}</span>
          {!isIndeterminate && <span>{Math.round(progress * 100)}%</span>}
        </div>
        <div className="h-2 rounded-full bg-stone-700 overflow-hidden">
          {isIndeterminate ? (
            <div className="h-full w-1/3 bg-primary-500 rounded-full animate-pulse" />
          ) : (
            <div
              className="h-full bg-primary-500 rounded-full transition-all duration-300"
              style={{ width: `${Math.round(progress * 100)}%` }}
            />
          )}
        </div>
        {(downloadedText || speedText || etaText) && (
          <div className="flex items-center justify-between text-xs opacity-50 mt-1.5">
            <span>{downloadedText}</span>
            <span>
              {speedText}
              {speedText && etaText ? ' · ' : ''}
              {etaText && `${etaText} remaining`}
            </span>
          </div>
        )}
      </div>

      {error && <p className="text-coral-400 text-sm mb-3 text-center">{error}</p>}

      {isTriggeringDownload && (
        <p className="text-xs opacity-50 text-center mb-3">Starting download...</p>
      )}

      <button
        onClick={() => onNext({ consentGiven: true, downloadStarted })}
        className="btn-primary w-full py-2.5 text-sm font-medium rounded-xl">
        {isReady ? 'Continue' : 'Continue in Background'}
      </button>
    </div>
  );
};

export default LocalAIStep;
