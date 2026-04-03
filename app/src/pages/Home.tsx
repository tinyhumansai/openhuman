import { useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import ConnectionIndicator from '../components/ConnectionIndicator';
import { useUser } from '../hooks/useUser';
import {
  isTauri,
  type LocalAiStatus,
  openhumanLocalAiDownload,
  openhumanLocalAiStatus,
} from '../utils/tauriCommands';

const progressFromStatus = (status: LocalAiStatus | null): number => {
  if (!status) return 0;
  if (typeof status.download_progress === 'number') {
    return Math.max(0, Math.min(1, status.download_progress));
  }
  switch (status.state) {
    case 'ready':
      return 1;
    case 'loading':
      return 0.92;
    case 'downloading':
      return 0.25;
    case 'installing':
      return 0.1;
    case 'idle':
      return 0;
    default:
      return 0;
  }
};

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
  if (typeof etaSeconds !== 'number' || !Number.isFinite(etaSeconds) || etaSeconds <= 0) {
    return '';
  }
  const mins = Math.floor(etaSeconds / 60);
  const secs = etaSeconds % 60;
  if (mins <= 0) return `${secs}s`;
  return `${mins}m ${secs.toString().padStart(2, '0')}s`;
};

const Home = () => {
  const { user } = useUser();
  const navigate = useNavigate();
  const userName = user?.firstName || 'User';
  const [localAiStatus, setLocalAiStatus] = useState<LocalAiStatus | null>(null);
  const [downloadBusy, setDownloadBusy] = useState(false);
  const autoRetryDoneRef = useRef(false);

  // Get greeting based on time
  const getGreeting = () => {
    const hour = new Date().getHours();
    if (hour < 12) return 'Good morning';
    if (hour < 18) return 'Good afternoon';
    return 'Good evening';
  };

  // Open in-app conversations window
  const handleStartCooking = async () => {
    navigate('/conversations');
  };

  useEffect(() => {
    if (!isTauri()) return;
    let mounted = true;
    const load = async () => {
      try {
        const status = await openhumanLocalAiStatus();
        if (mounted) {
          setLocalAiStatus(status.result);
          // Auto-retry bootstrap once if Ollama is degraded (install/server issue).
          if (status.result?.state === 'degraded' && !autoRetryDoneRef.current) {
            autoRetryDoneRef.current = true;
            void openhumanLocalAiDownload(true).catch(() => {});
          }
        }
      } catch {
        if (mounted) setLocalAiStatus(null);
      }
    };
    void load();
    const timer = setInterval(() => void load(), 2000);
    return () => {
      mounted = false;
      clearInterval(timer);
    };
  }, []);

  const modelProgress = useMemo(() => progressFromStatus(localAiStatus), [localAiStatus]);
  const isInstalling = localAiStatus?.state === 'installing';
  const indeterminateDownload =
    isInstalling ||
    (localAiStatus?.state === 'downloading' && typeof localAiStatus.download_progress !== 'number');
  const isInstallError =
    localAiStatus?.state === 'degraded' && localAiStatus?.error_category === 'install';
  const [showErrorDetail, setShowErrorDetail] = useState(false);
  const downloadedText =
    typeof localAiStatus?.downloaded_bytes === 'number'
      ? `${formatBytes(localAiStatus.downloaded_bytes)}${typeof localAiStatus?.total_bytes === 'number' ? ` / ${formatBytes(localAiStatus.total_bytes)}` : ''}`
      : '';
  const speedText =
    typeof localAiStatus?.download_speed_bps === 'number' && localAiStatus.download_speed_bps > 0
      ? `${formatBytes(localAiStatus.download_speed_bps)}/s`
      : '';
  const etaText = formatEta(localAiStatus?.eta_seconds);

  return (
    <div className="min-h-full flex flex-col items-center justify-center p-4">
      <div className="max-w-md w-full">
        {/* Main card */}
        <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-6 animate-fade-up">
          {/* Header row: logo + version + avatar */}
          <div className="flex items-center justify-between mb-8">
            <div className="w-9 h-9 bg-stone-900 rounded-lg flex items-center justify-center">
              <svg className="w-5 h-5 text-white" fill="currentColor" viewBox="0 0 24 24">
                <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" />
              </svg>
            </div>
            <span className="text-xs text-stone-400">web, 01</span>
            <button
              onClick={() => navigate('/settings')}
              className="w-9 h-9 rounded-full bg-stone-100 flex items-center justify-center hover:bg-stone-200 transition-colors"
              aria-label="Settings">
              <svg
                className="w-4 h-4 text-stone-500"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
                />
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
                />
              </svg>
            </button>
          </div>

          {/* Welcome title */}
          <h1 className="text-3xl font-bold text-stone-900 text-center mb-6">Welcome Onboard</h1>

          {/* Greeting */}
          <div className="text-center mb-3">
            <p className="text-lg font-medium text-stone-700">
              {getGreeting()}, {userName}
            </p>
          </div>

          {/* Connection status */}
          <div className="flex justify-center mb-3">
            <ConnectionIndicator />
          </div>

          {/* Description */}
          <p className="text-sm text-stone-500 text-center mb-6 leading-relaxed">
            Your device is now connected to the OpenHuman AI. Keep the app running to keep the
            connection alive. You can message your assistant with the button below.
          </p>

          {/* CTA button */}
          <button
            onClick={handleStartCooking}
            className="w-full py-3 bg-primary-500 hover:bg-primary-600 text-white font-medium rounded-xl transition-colors duration-200">
            Message OpenHuman
          </button>
        </div>

        {/* Local AI card (desktop only) */}
        {isTauri() && (
          <div className="mt-3 bg-white rounded-2xl shadow-soft border border-stone-200 px-4 py-4 text-left">
            <div className="flex items-center justify-between">
              <div className="text-[11px] uppercase tracking-wide text-stone-400">
                Local model runtime
              </div>
              <button
                onClick={() => navigate('/settings/local-model')}
                className="text-xs text-primary-500 hover:text-primary-600 transition-colors">
                Manage
              </button>
            </div>

            <div className="mt-2 flex items-center justify-between text-xs">
              <span className="text-stone-600">
                {localAiStatus?.model_id ?? 'gemma3:4b-it-qat'}
              </span>
              <span className="text-stone-700 capitalize">
                {localAiStatus?.state ?? 'starting'}
              </span>
            </div>

            <div className="mt-2 h-2 rounded-full bg-stone-100 overflow-hidden">
              <div
                className={`h-full bg-gradient-to-r from-primary-500 to-primary-400 transition-all duration-500 ${
                  indeterminateDownload ? 'animate-pulse' : ''
                }`}
                style={{
                  width: `${Math.round((indeterminateDownload ? 1 : modelProgress) * 100)}%`,
                }}
              />
            </div>

            <div className="mt-2 flex items-center justify-between gap-2 text-[11px] text-stone-400">
              <span>
                {isInstalling
                  ? 'Installing Ollama runtime...'
                  : indeterminateDownload
                    ? 'Downloading...'
                    : `${Math.round(modelProgress * 100)}%`}
              </span>
              {downloadedText && (
                <span className="truncate text-stone-500" title={downloadedText}>
                  {downloadedText}
                </span>
              )}
              {speedText && <span className="text-primary-500">{speedText}</span>}
              {etaText && <span className="text-primary-600">ETA {etaText}</span>}
            </div>
            {localAiStatus?.warning && (
              <div
                className="mt-1 text-[11px] text-stone-400 truncate"
                title={localAiStatus.warning}>
                {localAiStatus.warning}
              </div>
            )}

            {isInstallError && localAiStatus?.error_detail && (
              <div className="mt-2">
                <button
                  onClick={() => setShowErrorDetail(v => !v)}
                  className="text-[11px] text-coral-500 hover:text-coral-600 underline">
                  {showErrorDetail ? 'Hide error details' : 'Show error details'}
                </button>
                {showErrorDetail && (
                  <pre className="mt-1 max-h-32 overflow-auto rounded bg-stone-50 p-2 text-[10px] text-coral-600 leading-tight whitespace-pre-wrap break-words">
                    {localAiStatus.error_detail}
                  </pre>
                )}
                <p className="mt-1 text-[11px] text-stone-400">
                  Install Ollama manually from{' '}
                  <a
                    href="https://ollama.com"
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-primary-500 hover:text-primary-600 underline">
                    ollama.com
                  </a>{' '}
                  then set its path in{' '}
                  <button
                    onClick={() => navigate('/settings/local-model')}
                    className="text-primary-500 hover:text-primary-600 underline">
                    Settings
                  </button>
                  .
                </p>
              </div>
            )}

            <div className="mt-2 flex items-center gap-2">
              <button
                onClick={async () => {
                  setDownloadBusy(true);
                  try {
                    await openhumanLocalAiDownload(false);
                    const status = await openhumanLocalAiStatus();
                    setLocalAiStatus(status.result);
                  } finally {
                    setDownloadBusy(false);
                  }
                }}
                disabled={downloadBusy}
                className="rounded-md bg-primary-500 px-2.5 py-1.5 text-[11px] font-medium text-white hover:bg-primary-600 disabled:opacity-60">
                {downloadBusy ? 'Working...' : 'Bootstrap'}
              </button>
              <button
                onClick={async () => {
                  setDownloadBusy(true);
                  try {
                    await openhumanLocalAiDownload(true);
                    const status = await openhumanLocalAiStatus();
                    setLocalAiStatus(status.result);
                  } finally {
                    setDownloadBusy(false);
                  }
                }}
                disabled={downloadBusy}
                className="rounded-md border border-stone-200 px-2.5 py-1.5 text-[11px] font-medium text-stone-600 hover:border-stone-300 disabled:opacity-60">
                Re-bootstrap
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};

export default Home;
