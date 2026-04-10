import { useEffect, useMemo, useState } from 'react';

import {
  formatBytes,
  formatEta,
  progressFromDownloads,
  progressFromStatus,
} from '../../../utils/localAiHelpers';
import {
  type ApplyPresetResult,
  type LocalAiDownloadsProgress,
  type LocalAiStatus,
  openhumanLocalAiApplyPreset,
  openhumanLocalAiDownload,
  openhumanLocalAiDownloadAllAssets,
  openhumanLocalAiDownloadsProgress,
  openhumanLocalAiPresets,
  openhumanLocalAiStatus,
  type PresetsResponse,
} from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import DeviceCapabilitySection from './local-model/DeviceCapabilitySection';

const formatRamGb = (bytes: number): string => {
  const gb = bytes / (1024 * 1024 * 1024);
  return gb >= 10 ? `${Math.round(gb)} GB` : `${gb.toFixed(1)} GB`;
};

const LocalModelPanel = () => {
  const { navigateBack, navigateToSettings, breadcrumbs } = useSettingsNavigation();
  const [status, setStatus] = useState<LocalAiStatus | null>(null);
  const [downloads, setDownloads] = useState<LocalAiDownloadsProgress | null>(null);
  const [statusError, setStatusError] = useState<string>('');
  const [isTriggeringDownload, setIsTriggeringDownload] = useState(false);
  const [bootstrapMessage, setBootstrapMessage] = useState<string>('');

  const [presetsData, setPresetsData] = useState<PresetsResponse | null>(null);
  const [presetsLoading, setPresetsLoading] = useState(true);
  const [isApplyingPreset, setIsApplyingPreset] = useState(false);
  const [presetError, setPresetError] = useState('');
  const [presetSuccess, setPresetSuccess] = useState<ApplyPresetResult | null>(null);

  const progress = useMemo(() => {
    const downloadProgress = progressFromDownloads(downloads);
    if (downloadProgress != null) return downloadProgress;
    return progressFromStatus(status);
  }, [downloads, status]);
  const currentState = downloads?.state ?? status?.state;
  const isInstalling = currentState === 'installing';
  const isIndeterminateDownload =
    isInstalling ||
    (currentState === 'downloading' &&
      typeof downloads?.progress !== 'number' &&
      typeof status?.download_progress !== 'number');
  const downloadedBytes = downloads?.downloaded_bytes ?? status?.downloaded_bytes;
  const totalBytes = downloads?.total_bytes ?? status?.total_bytes;
  const speedBps = downloads?.speed_bps ?? status?.download_speed_bps;
  const etaSeconds = downloads?.eta_seconds ?? status?.eta_seconds;

  const loadStatus = async () => {
    try {
      const [statusResponse, downloadsResponse] = await Promise.all([
        openhumanLocalAiStatus(),
        openhumanLocalAiDownloadsProgress(),
      ]);
      setStatus(statusResponse.result);
      setDownloads(downloadsResponse.result);
      setStatusError('');
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to read local model status';
      setStatusError(message);
      setStatus(null);
      setDownloads(null);
    }
  };

  const loadPresets = async () => {
    setPresetsLoading(true);
    try {
      const data = await openhumanLocalAiPresets();
      setPresetsData(data);
      setPresetError('');
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to load presets';
      setPresetError(msg);
    } finally {
      setPresetsLoading(false);
    }
  };

  const applyPreset = async (tier: string) => {
    setIsApplyingPreset(true);
    setPresetError('');
    setPresetSuccess(null);
    try {
      const result = await openhumanLocalAiApplyPreset(tier);
      setPresetSuccess(result);
      await loadPresets();
      await loadStatus();
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to apply preset';
      setPresetError(msg);
    } finally {
      setIsApplyingPreset(false);
    }
  };

  useEffect(() => {
    void loadStatus();
    void loadPresets();
    const timer = setInterval(() => {
      void loadStatus();
    }, 1500);
    return () => clearInterval(timer);
  }, []);

  const triggerDownload = async (force: boolean) => {
    setIsTriggeringDownload(true);
    setStatusError('');
    setBootstrapMessage('');
    try {
      await openhumanLocalAiDownload(force);
      await openhumanLocalAiDownloadAllAssets(force);
      const freshStatus = await openhumanLocalAiStatus();
      setStatus(freshStatus.result);
      if (freshStatus.result?.state === 'ready') {
        setBootstrapMessage(force ? 'Re-bootstrap complete' : 'Models verified');
      }
      setTimeout(() => setBootstrapMessage(''), 3000);
    } catch (err) {
      const message =
        err instanceof Error ? err.message : 'Failed to trigger local model bootstrap';
      setStatusError(message);
    } finally {
      setIsTriggeringDownload(false);
    }
  };

  return (
    <div>
      <SettingsHeader
        title="Local AI Model"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        <DeviceCapabilitySection
          presetsData={presetsData}
          presetsLoading={presetsLoading}
          presetError={presetError}
          presetSuccess={presetSuccess}
          isApplyingPreset={isApplyingPreset}
          onApplyPreset={tier => void applyPreset(tier)}
          formatRamGb={formatRamGb}
        />

        {/* Simplified download status */}
        <section className="bg-stone-50 rounded-lg border border-stone-200 p-4 space-y-3">
          <h3 className="text-sm font-semibold text-stone-900">Model Status</h3>

          <div className="text-sm text-stone-600">
            State:{' '}
            <span
              className={`font-medium ${
                currentState === 'ready'
                  ? 'text-green-600'
                  : currentState === 'downloading' || currentState === 'installing'
                    ? 'text-primary-600'
                    : currentState === 'degraded'
                      ? 'text-amber-700'
                      : 'text-stone-700'
              }`}>
              {currentState ?? 'unknown'}
            </span>
          </div>

          {(currentState === 'downloading' || isInstalling) && (
            <div className="space-y-2">
              <div className="w-full h-2 rounded-full bg-stone-200 overflow-hidden">
                {isIndeterminateDownload ? (
                  <div className="h-full bg-primary-500 animate-pulse rounded-full w-1/2" />
                ) : (
                  <div
                    className="h-full bg-primary-500 rounded-full transition-all"
                    style={{ width: `${String(Math.min(progress ?? 0, 100))}%` }}
                  />
                )}
              </div>
              <div className="flex justify-between text-xs text-stone-500">
                <span>
                  {typeof downloadedBytes === 'number'
                    ? `${formatBytes(downloadedBytes)}${typeof totalBytes === 'number' ? ` / ${formatBytes(totalBytes)}` : ''}`
                    : ''}
                </span>
                <span>
                  {typeof speedBps === 'number' && speedBps > 0 ? `${formatBytes(speedBps)}/s` : ''}
                  {etaSeconds ? ` · ${formatEta(etaSeconds)}` : ''}
                </span>
              </div>
            </div>
          )}

          {bootstrapMessage && <div className="text-xs text-green-700">{bootstrapMessage}</div>}

          <div className="flex gap-2">
            <button
              type="button"
              onClick={() => void triggerDownload(false)}
              disabled={isTriggeringDownload}
              className="rounded-lg border border-primary-400 bg-primary-50 px-3 py-2 text-sm text-primary-700 disabled:opacity-50">
              {isTriggeringDownload ? 'Downloading…' : 'Download Models'}
            </button>
            <button
              type="button"
              onClick={() => void loadStatus()}
              className="rounded-lg border border-stone-300 bg-stone-100 px-3 py-2 text-sm text-stone-700">
              Refresh
            </button>
          </div>

          {statusError && (
            <div className="rounded-md border border-red-200 bg-red-50 p-3 text-xs text-red-600">
              {statusError}
            </div>
          )}
        </section>

        <button
          type="button"
          onClick={() => navigateToSettings('local-model-debug')}
          className="flex items-center gap-1.5 text-xs text-stone-400 hover:text-stone-600 transition-colors">
          Advanced settings
          <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
          </svg>
        </button>
      </div>
    </div>
  );
};

export default LocalModelPanel;
