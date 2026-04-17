import { useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import ConnectionIndicator from '../components/ConnectionIndicator';
import { useUser } from '../hooks/useUser';
import {
  bootstrapLocalAiWithRecommendedPreset,
  ensureRecommendedLocalAiPresetIfNeeded,
  triggerLocalAiAssetBootstrap,
} from '../utils/localAiBootstrap';
import { formatBytes, formatEta, progressFromStatus } from '../utils/localAiHelpers';
import {
  isTauri,
  type LocalAiAssetsStatus,
  type LocalAiStatus,
  openhumanLocalAiAssetsStatus,
  openhumanLocalAiStatus,
} from '../utils/tauriCommands';

const Home = () => {
  const { user } = useUser();
  const navigate = useNavigate();
  const userName = user?.firstName || 'User';
  const [localAiStatus, setLocalAiStatus] = useState<LocalAiStatus | null>(null);
  const [localAiAssets, setLocalAiAssets] = useState<LocalAiAssetsStatus | null>(null);
  const [downloadBusy, setDownloadBusy] = useState(false);
  const [bootstrapMessage, setBootstrapMessage] = useState<string>('');
  const autoRetryDoneRef = useRef(false);
  const initialBootstrapHandledRef = useRef(false);
  const initialBootstrapInFlightRef = useRef(false);
  const initialBootstrapPendingDownloadRef = useRef(false);
  const initialBootstrapAttemptsRef = useRef(0);

  // Get greeting based on time
  const getGreeting = () => {
    const hour = new Date().getHours();
    if (hour < 12) return 'Good morning';
    if (hour < 18) return 'Good afternoon';
    return 'Good evening';
  };

  // Open in-app chat.
  const handleStartCooking = async () => {
    navigate('/chat');
  };

  const refreshLocalAiStatus = async () => {
    const status = await openhumanLocalAiStatus();
    setLocalAiStatus(status.result);
    return status.result;
  };

  const runManualBootstrap = async (force: boolean) => {
    setDownloadBusy(true);
    setBootstrapMessage('');
    try {
      await bootstrapLocalAiWithRecommendedPreset(
        force,
        force ? '[Home re-bootstrap]' : '[Home manual bootstrap]'
      );
      const freshStatus = await refreshLocalAiStatus();
      if (freshStatus?.state === 'ready') {
        setBootstrapMessage(force ? 'Re-bootstrap complete' : 'Local AI is ready');
      } else if (freshStatus?.state === 'degraded') {
        setBootstrapMessage('Bootstrap failed — check warning below');
      }
      setTimeout(() => setBootstrapMessage(''), 3000);
    } catch (error) {
      console.warn('[Home] manual Local AI bootstrap failed:', error);
      setBootstrapMessage('Bootstrap failed');
      setTimeout(() => setBootstrapMessage(''), 3000);
    } finally {
      setDownloadBusy(false);
    }
  };

  useEffect(() => {
    if (!isTauri()) return;
    const MAX_INITIAL_BOOTSTRAP_ATTEMPTS = 3;
    let mounted = true;
    const load = async () => {
      try {
        const [status, assets] = await Promise.all([
          openhumanLocalAiStatus(),
          openhumanLocalAiAssetsStatus().catch(err => {
            console.warn('[Home] failed to load local AI assets status:', err);
            return null;
          }),
        ]);
        if (mounted) {
          setLocalAiStatus(status.result);
          setLocalAiAssets(assets?.result ?? null);

          // Auto-retry bootstrap once if Ollama is degraded (install/server issue).
          if (status.result?.state === 'degraded' && !autoRetryDoneRef.current) {
            autoRetryDoneRef.current = true;
            console.debug('[Home] local AI is degraded; scheduling a one-time re-bootstrap');
            void bootstrapLocalAiWithRecommendedPreset(true, '[Home degraded auto-retry]').catch(
              error => {
                autoRetryDoneRef.current = false;
                console.warn('[Home] degraded local AI re-bootstrap failed:', error);
              }
            );
          }

          if (
            status.result?.state === 'idle' &&
            !initialBootstrapHandledRef.current &&
            !initialBootstrapInFlightRef.current
          ) {
            initialBootstrapInFlightRef.current = true;
            console.debug('[Home] local AI is idle; checking first-run preset selection');
            void ensureRecommendedLocalAiPresetIfNeeded('[Home first-run]')
              .then(async preset => {
                const shouldTriggerBootstrap =
                  !preset.hadSelectedTier || initialBootstrapPendingDownloadRef.current;

                if (!shouldTriggerBootstrap) {
                  console.debug(
                    '[Home] skipping automatic first-run bootstrap because a tier is already selected'
                  );
                  initialBootstrapHandledRef.current = true;
                  return;
                }

                initialBootstrapPendingDownloadRef.current = true;
                console.debug(
                  '[Home] selected recommended preset for first-run bootstrap',
                  JSON.stringify({
                    recommendedTier: preset.recommendedTier,
                    hadSelectedTier: preset.hadSelectedTier,
                  })
                );
                await triggerLocalAiAssetBootstrap(false, '[Home first-run]');
                initialBootstrapPendingDownloadRef.current = false;
                initialBootstrapHandledRef.current = true;
                initialBootstrapAttemptsRef.current = 0;
              })
              .catch(error => {
                initialBootstrapAttemptsRef.current += 1;
                const attempts = initialBootstrapAttemptsRef.current;
                if (attempts >= MAX_INITIAL_BOOTSTRAP_ATTEMPTS) {
                  initialBootstrapPendingDownloadRef.current = false;
                  initialBootstrapHandledRef.current = true;
                  console.warn(
                    '[Home] first-run local AI bootstrap failed permanently; stopping retries',
                    { attempts, error }
                  );
                  return;
                }
                console.warn('[Home] first-run local AI bootstrap failed:', error);
              })
              .finally(() => {
                initialBootstrapInFlightRef.current = false;
              });
          }
        }
      } catch (error) {
        console.warn('[Home] failed to load local AI status:', error);
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
  // Hide the Local Model Runtime card once every capability's model file is
  // present on disk. We use `assets_status` (which inspects the filesystem)
  // instead of the in-memory `LocalAiStatus` sub-states, because the latter
  // stay at `idle` until a capability is first exercised — even when the
  // underlying model has already been downloaded.
  //
  // A capability is considered "done" when its asset state is:
  //   - `ready`    → model file exists on disk
  //   - `disabled` → not applicable for the selected preset
  //   - `ondemand` → vision preset intentionally defers download until first use
  const allModelsDownloaded = useMemo(() => {
    if (!localAiStatus || !localAiAssets) return false;
    if (localAiStatus.state !== 'ready') return false;
    const isDone = (state: string | undefined | null): boolean =>
      state === 'ready' || state === 'disabled' || state === 'ondemand';

    return (
      isDone(localAiAssets.chat?.state) &&
      isDone(localAiAssets.vision?.state) &&
      isDone(localAiAssets.embedding?.state) &&
      isDone(localAiAssets.stt?.state) &&
      isDone(localAiAssets.tts?.state)
    );
  }, [localAiStatus, localAiAssets]);

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
          {/* Header row: logo + version + settings */}
          <div className="flex items-center justify-between mb-8">
            <div className="w-9 h-9 bg-stone-900 rounded-lg flex items-center justify-center">
              <svg className="w-5 h-5 text-white" fill="currentColor" viewBox="0 0 24 24">
                <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" />
              </svg>
            </div>
            <span className="text-xs text-stone-400">web, 01</span>
            <button
              onClick={() => navigate('/settings/messaging')}
              className="w-9 h-9 rounded-full bg-stone-100 flex items-center justify-center hover:bg-stone-200 transition-colors"
              aria-label="Notifications">
              <svg
                className="w-4 h-4 text-stone-500"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9"
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

        {/* Local AI card (desktop only) — hidden once all models are fully downloaded */}
        {isTauri() && !allModelsDownloaded && (
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
              {localAiStatus?.state === 'ready' ? (
                <span className="inline-flex items-center gap-1 rounded-md bg-green-50 px-2.5 py-1.5 text-[11px] font-medium text-green-700 border border-green-200">
                  <svg className="h-3 w-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M5 13l4 4L19 7"
                    />
                  </svg>
                  Running
                </span>
              ) : (
                <button
                  onClick={() => void runManualBootstrap(false)}
                  disabled={downloadBusy}
                  className="rounded-md bg-primary-500 px-2.5 py-1.5 text-[11px] font-medium text-white hover:bg-primary-600 disabled:opacity-60">
                  {downloadBusy
                    ? 'Working...'
                    : localAiStatus?.state === 'degraded'
                      ? 'Retry'
                      : 'Bootstrap'}
                </button>
              )}
              <button
                onClick={() => void runManualBootstrap(true)}
                disabled={downloadBusy}
                className="rounded-md border border-stone-200 px-2.5 py-1.5 text-[11px] font-medium text-stone-600 hover:border-stone-300 disabled:opacity-60">
                {downloadBusy ? 'Working...' : 'Re-bootstrap'}
              </button>
              {bootstrapMessage && (
                <span className="text-[11px] text-green-600 animate-fade-up">
                  {bootstrapMessage}
                </span>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
};

export default Home;
