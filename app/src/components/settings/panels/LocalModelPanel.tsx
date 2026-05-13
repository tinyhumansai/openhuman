import { useEffect, useMemo, useState } from 'react';

import { triggerLocalAiAssetBootstrap } from '../../../utils/localAiBootstrap';
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
  openhumanGetConfig,
  openhumanLocalAiApplyPreset,
  openhumanLocalAiDownload,
  openhumanLocalAiDownloadAllAssets,
  openhumanLocalAiDownloadsProgress,
  openhumanLocalAiPresets,
  openhumanLocalAiStatus,
  openhumanUpdateLocalAiSettings,
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
  const [presetError, setPresetError] = useState('');
  const [presetSuccess, setPresetSuccess] = useState<ApplyPresetResult | null>(null);

  const [usageFlags, setUsageFlags] = useState<{
    runtime_enabled: boolean;
    usage_embeddings: boolean;
    usage_heartbeat: boolean;
    usage_learning_reflection: boolean;
    usage_subconscious: boolean;
  }>({
    runtime_enabled: false,
    usage_embeddings: false,
    usage_heartbeat: false,
    usage_learning_reflection: false,
    usage_subconscious: false,
  });
  const [usageError, setUsageError] = useState('');
  const [usageSaving, setUsageSaving] = useState(false);

  const progress = useMemo(() => {
    const downloadProgress = progressFromDownloads(downloads);
    if (downloadProgress != null) return downloadProgress;
    return progressFromStatus(status);
  }, [downloads, status]);
  const currentState = downloads?.state ?? status?.state;
  const runtimeEnabled = usageFlags.runtime_enabled;
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

  const loadUsage = async () => {
    try {
      const snap = await openhumanGetConfig();
      const localAi = (snap.result?.config?.local_ai ?? {}) as Record<string, unknown>;
      const usage = (localAi.usage ?? {}) as Record<string, unknown>;
      setUsageFlags({
        runtime_enabled: Boolean(localAi.runtime_enabled),
        usage_embeddings: Boolean(usage.embeddings),
        usage_heartbeat: Boolean(usage.heartbeat),
        usage_learning_reflection: Boolean(usage.learning_reflection),
        usage_subconscious: Boolean(usage.subconscious),
      });
      setUsageError('');
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to load local AI flags';
      setUsageError(msg);
    }
  };

  const updateUsage = async (patch: Partial<typeof usageFlags>) => {
    const next = { ...usageFlags, ...patch };
    setUsageFlags(next);
    setUsageSaving(true);
    setUsageError('');
    try {
      await openhumanUpdateLocalAiSettings(patch);
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to save local AI flags';
      setUsageError(msg);
      void loadUsage();
    } finally {
      setUsageSaving(false);
    }
  };

  useEffect(() => {
    const initialLoad = window.setTimeout(() => {
      void loadStatus();
      void loadPresets();
      void loadUsage();
    }, 0);
    const timer = window.setInterval(() => {
      void loadStatus();
    }, 1500);
    return () => {
      window.clearTimeout(initialLoad);
      window.clearInterval(timer);
    };
  }, []);

  const triggerDownload = async (force: boolean) => {
    if (!runtimeEnabled) return;
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

  /**
   * Install-Ollama entry point used by the locked tier picker and the
   * runtime-status install CTA.
   *
   * Three preconditions need to be flipped before `download_all_models`
   * will actually run on the core side:
   *   - `runtime_enabled = true`   (cloud-fallback override off)
   *   - `selected_tier`            (anything other than "disabled")
   *   - `opt_in_confirmed = true`  (so bootstrap doesn't hard-override
   *                                 to disabled in `config_with_recommended_tier_if_unselected`)
   *
   * `apply_preset(<real tier>)` sets all three in one save. We call it
   * **unconditionally** here rather than going through
   * `ensureRecommendedLocalAiPresetIfNeeded`, because that helper
   * short-circuits when `selected_tier` is already set — which is exactly
   * the case for users who previously picked "Disabled (cloud fallback)"
   * and now want to switch on local AI. Without the explicit apply,
   * runtime_enabled stays false, `download_all_models` returns
   * `local ai is disabled`, the task marks the service degraded, and the
   * UI silently bounces back to idle.
   */
  const triggerInstallWithRecommendedTier = async () => {
    setIsTriggeringDownload(true);
    setStatusError('');
    setBootstrapMessage('');
    try {
      const presetsResult = presetsData ?? (await openhumanLocalAiPresets());
      const tier = presetsResult.recommended_tier || 'ram_2_4gb';
      if (tier === 'disabled') {
        throw new Error('Cannot install Ollama for the "disabled" tier — pick a local tier first.');
      }
      await openhumanLocalAiApplyPreset(tier);
      await triggerLocalAiAssetBootstrap(true);
      await loadPresets();
      const freshStatus = await openhumanLocalAiStatus();
      setStatus(freshStatus.result);
      setBootstrapMessage('Install started — Ollama and models are downloading');
      setTimeout(() => setBootstrapMessage(''), 4000);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to start Ollama install';
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
          formatRamGb={formatRamGb}
          ollamaAvailable={downloads?.ollama_available ?? true}
          onTriggerOllamaInstall={() => void triggerInstallWithRecommendedTier()}
          isTriggeringInstall={isTriggeringDownload}
          installState={status?.state}
          installWarning={status?.warning}
          installError={status?.error_detail}
          onPresetApplied={result => {
            setPresetSuccess(result);
            void loadPresets();
            void loadStatus();
          }}
        />

        {/*
          Simplified Model Status — only meaningful AFTER Ollama is on disk.
          Before that, every readout here ("idle"/"missing", Re-bootstrap
          button, refresh) is noise: there's no runtime to inspect and the
          right call-to-action is "Install Ollama" up top in the tier-picker
          banner. Hide the whole section to keep the UI progressive:
          1) Ollama missing            → only the install CTA (above)
          2) Ollama installing         → CTA flips to blue progress (above)
          3) Ollama installed onward   → this section appears with model state
        */}
        {(downloads?.ollama_available ?? true) && (
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
                    {typeof speedBps === 'number' && speedBps > 0
                      ? `${formatBytes(speedBps)}/s`
                      : ''}
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
                disabled={!runtimeEnabled || isTriggeringDownload}
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
        )}

        <section className="bg-stone-50 rounded-lg border border-stone-200 p-4 space-y-3">
          <div>
            <h3 className="text-sm font-semibold text-stone-900">Usage</h3>
            <p className="text-xs text-stone-500 mt-0.5">
              Choose which subsystems run on the local model. Anything off uses the cloud.
            </p>
          </div>

          <label className="flex items-start gap-3 cursor-pointer">
            <input
              type="checkbox"
              className="mt-0.5"
              checked={usageFlags.runtime_enabled}
              disabled={usageSaving}
              onChange={e => void updateUsage({ runtime_enabled: e.target.checked })}
            />
            <div>
              <div className="text-sm text-stone-900">Enable local AI runtime</div>
              <div className="text-xs text-stone-500">
                Master switch. Off by default — Ollama stays idle. When on, the tree summarizer,
                screen intelligence, and autocomplete always use the local model.
              </div>
            </div>
          </label>

          <div className={`space-y-2 pl-6 ${usageFlags.runtime_enabled ? '' : 'opacity-50'}`}>
            {(
              [
                {
                  key: 'usage_embeddings' as const,
                  label: 'Embeddings',
                  hint: 'Generate memory embeddings locally instead of in the cloud.',
                },
                {
                  key: 'usage_heartbeat' as const,
                  label: 'Heartbeat',
                  hint: 'Run heartbeat reasoning locally.',
                },
                {
                  key: 'usage_learning_reflection' as const,
                  label: 'Learning / reflection',
                  hint: 'Run learning and reflection passes locally.',
                },
                {
                  key: 'usage_subconscious' as const,
                  label: 'Subconscious',
                  hint: 'Run subconscious evaluation locally.',
                },
              ] as const
            ).map(({ key, label, hint }) => (
              <label key={key} className="flex items-start gap-3 cursor-pointer">
                <input
                  type="checkbox"
                  className="mt-0.5"
                  checked={usageFlags[key]}
                  disabled={!usageFlags.runtime_enabled || usageSaving}
                  onChange={e => void updateUsage({ [key]: e.target.checked })}
                />
                <div>
                  <div className="text-sm text-stone-900">{label}</div>
                  <div className="text-xs text-stone-500">{hint}</div>
                </div>
              </label>
            ))}
          </div>

          {usageError && (
            <div className="rounded-md border border-red-200 bg-red-50 p-3 text-xs text-red-600">
              {usageError}
            </div>
          )}
        </section>

        <button
          type="button"
          onClick={() => {
            if (runtimeEnabled) navigateToSettings('local-model-debug');
          }}
          disabled={!runtimeEnabled}
          className="flex items-center gap-1.5 text-xs text-stone-400 hover:text-stone-600 transition-colors disabled:opacity-50 disabled:hover:text-stone-400">
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
