import { useState } from 'react';

import { useT } from '../../../../lib/i18n/I18nContext';
import {
  type ApplyPresetResult,
  openhumanLocalAiApplyPreset,
  type PresetsResponse,
} from '../../../../utils/tauriCommands';

interface DeviceCapabilitySectionProps {
  presetsData: PresetsResponse | null;
  presetsLoading: boolean;
  presetError: string;
  presetSuccess: ApplyPresetResult | null;
  formatRamGb: (bytes: number) => string;
  onPresetApplied?: (result: ApplyPresetResult) => void;
  /**
   * When `false`, the external Ollama runtime isn't reachable yet. Local tiers
   * stay disabled until the user runs Ollama themselves. The "Disabled (cloud
   * fallback)" option stays enabled since it doesn't depend on Ollama.
   */
  ollamaAvailable?: boolean;
  onTriggerOllamaInstall?: () => void;
  isTriggeringInstall?: boolean;
  installState?: string;
  installWarning?: string | null;
  installError?: string | null;
}

const DISABLED_TIER_ID = 'disabled';

const DeviceCapabilitySection = ({
  presetsData,
  presetsLoading,
  presetError,
  presetSuccess,
  formatRamGb,
  onPresetApplied,
  ollamaAvailable = true,
  onTriggerOllamaInstall,
  isTriggeringInstall = false,
  installState,
  installWarning,
  installError,
}: DeviceCapabilitySectionProps) => {
  const { t } = useT();
  void onTriggerOllamaInstall;
  void isTriggeringInstall;
  void installState;
  void installWarning;
  void installError;
  const installInProgress = false;
  const installFailed = false;
  const [applying, setApplying] = useState<string | null>(null);
  const [applyError, setApplyError] = useState<string>('');
  const [applySuccess, setApplySuccess] = useState<ApplyPresetResult | null>(null);

  const isDisabledActive = presetsData ? presetsData.local_ai_enabled === false : false;

  const handleApply = async (tierId: string) => {
    setApplying(tierId);
    setApplyError('');
    try {
      const result = await openhumanLocalAiApplyPreset(tierId);
      setApplySuccess(result);
      onPresetApplied?.(result);
    } catch (err) {
      const msg =
        err instanceof Error
          ? err.message
          : t('settings.localModel.deviceCapability.failedToApplyPreset');
      setApplyError(msg);
    } finally {
      setApplying(null);
    }
  };

  return (
    <section className="space-y-3">
      <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
        {t('settings.localModel.deviceCapability.modelTier')}
      </h3>

      {presetsLoading && !presetsData && (
        <div className="bg-stone-50 dark:bg-neutral-800/60 rounded-lg border border-stone-200 dark:border-neutral-800 p-4 text-sm text-stone-500 dark:text-neutral-400 animate-pulse">
          {t('settings.localModel.deviceCapability.loadingDeviceInfo')}
        </div>
      )}
      {!presetsLoading && !presetsData && presetError && (
        <div className="bg-red-50 dark:bg-red-500/10 rounded-lg border border-red-300 dark:border-red-500/40 p-4 text-sm text-red-600 dark:text-red-300">
          {t('settings.localModel.deviceCapability.couldNotLoadPresets')} {presetError}
        </div>
      )}

      {presetsData?.device && (
        <div className="bg-stone-50 dark:bg-neutral-800/60 rounded-lg border border-stone-200 dark:border-neutral-800 p-3">
          <div className="grid grid-cols-3 gap-3 text-xs">
            <div>
              <div className="text-stone-500 dark:text-neutral-400 uppercase tracking-wide">
                {t('settings.localModel.deviceCapability.ram')}
              </div>
              <div className="text-stone-800 dark:text-neutral-100 mt-0.5 font-medium">
                {formatRamGb(presetsData.device.total_ram_bytes)}
              </div>
            </div>
            <div>
              <div className="text-stone-500 dark:text-neutral-400 uppercase tracking-wide">
                {t('settings.localModel.deviceCapability.cpu')}
              </div>
              <div
                className="text-stone-800 dark:text-neutral-100 mt-0.5 font-medium truncate"
                title={presetsData.device.cpu_brand}>
                {t('settings.localModel.deviceCapability.cores').replace(
                  '{count}',
                  String(presetsData.device.cpu_count)
                )}
              </div>
            </div>
            <div>
              <div className="text-stone-500 dark:text-neutral-400 uppercase tracking-wide">
                {t('settings.localModel.deviceCapability.gpu')}
              </div>
              <div
                className="text-stone-800 dark:text-neutral-100 mt-0.5 font-medium truncate"
                title={presetsData.device.gpu_description ?? undefined}>
                {presetsData.device.has_gpu
                  ? (presetsData.device.gpu_description ??
                    t('settings.localModel.deviceCapability.detected'))
                  : t('settings.localModel.deviceCapability.notDetected')}
              </div>
            </div>
          </div>
        </div>
      )}

      {presetsData && !ollamaAvailable && (
        <div
          className={`rounded-lg border p-3 space-y-2 ${
            installFailed
              ? 'border-red-300 dark:border-red-500/40 bg-red-50 dark:bg-red-500/10'
              : installInProgress
                ? 'border-blue-300 dark:border-blue-500/40 bg-blue-50 dark:bg-blue-500/10'
                : 'border-amber-300 dark:border-amber-500/40 bg-amber-50 dark:bg-amber-500/10'
          }`}>
          {installInProgress ? (
            <>
              <div className="flex items-center gap-2">
                <div className="h-3 w-3 rounded-full border-2 border-blue-500 border-t-transparent animate-spin" />
                <div className="text-sm font-semibold text-blue-900">
                  {t('settings.localModel.deviceCapability.installingOllama')}
                  {installState === 'downloading'
                    ? ` (${t('settings.localModel.deviceCapability.downloadingModels')})`
                    : '…'}
                </div>
              </div>
              <div className="text-xs text-blue-800 dark:text-blue-200">
                {installWarning ?? t('settings.localModel.deviceCapability.downloadingSetupDesc')}
              </div>
              <div className="h-1.5 rounded-full bg-blue-200 dark:bg-blue-500/30 overflow-hidden">
                <div className="h-full w-1/3 bg-blue-500 animate-pulse" />
              </div>
            </>
          ) : installFailed ? (
            <>
              <div className="text-sm font-semibold text-red-900">
                {t('settings.localModel.deviceCapability.installFailed')}
              </div>
              <div className="text-xs text-red-800 dark:text-red-200">
                {installWarning ?? t('settings.localModel.deviceCapability.installFailedDesc')}
              </div>
              {installError && (
                <pre className="max-h-40 overflow-auto rounded bg-red-100 dark:bg-red-500/20 border border-red-200 dark:border-red-500/30 p-2 text-[10px] text-red-700 dark:text-red-300 leading-tight whitespace-pre-wrap break-words">
                  {installError}
                </pre>
              )}
              <div className="flex items-center gap-2 pt-1">
                {onTriggerOllamaInstall && (
                  <button
                    type="button"
                    onClick={onTriggerOllamaInstall}
                    disabled={isTriggeringInstall}
                    className="px-3 py-1.5 text-xs rounded-md bg-red-600 hover:bg-red-700 disabled:opacity-60 text-white font-medium">
                    {isTriggeringInstall
                      ? t('settings.localModel.deviceCapability.retrying')
                      : t('settings.localModel.deviceCapability.retryInstall')}
                  </button>
                )}
                <a
                  href="https://ollama.com"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="px-3 py-1.5 text-xs rounded-md border border-red-300 dark:border-red-500/40 hover:border-red-400 text-red-800 dark:text-red-200">
                  {t('settings.localModel.status.installManually')}
                </a>
              </div>
            </>
          ) : (
            <>
              <div className="text-xs text-amber-800 dark:text-amber-200">
                <span className="font-semibold text-amber-900">
                  {t('settings.localModel.deviceCapability.installFirst')}
                </span>{' '}
                {t('settings.localModel.deviceCapability.installFirstDesc')}
              </div>
              <div className="flex items-center gap-2">
                <a
                  href="https://ollama.com"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="px-3 py-1.5 text-xs rounded-md border border-amber-300 dark:border-amber-500/40 hover:border-amber-400 text-amber-800 dark:text-amber-200">
                  {t('settings.localModel.status.ollamaDocs')}
                </a>
              </div>
            </>
          )}
        </div>
      )}

      {presetsData && (
        <div className="space-y-2">
          {/* Disabled — Cloud fallback card (always available, recommended on low-RAM) */}
          <button
            type="button"
            onClick={() => void handleApply(DISABLED_TIER_ID)}
            disabled={applying !== null}
            className={`w-full text-left rounded-lg border p-3 transition-colors ${
              isDisabledActive
                ? 'border-primary-400 bg-primary-50 dark:bg-primary-500/10'
                : 'border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800'
            } ${applying !== null ? 'opacity-60 cursor-not-allowed' : 'cursor-pointer'}`}>
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <span className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                  {t('settings.localModel.deviceCapability.disabled')}
                </span>
                {isDisabledActive && (
                  <span className="px-1.5 py-0.5 text-[10px] font-medium rounded bg-primary-50 dark:bg-primary-500/10 text-primary-600 dark:text-primary-300 uppercase tracking-wide">
                    {t('settings.localModel.deviceCapability.active')}
                  </span>
                )}
                {(presetsData.recommend_disabled || !ollamaAvailable) && !isDisabledActive && (
                  <span className="px-1.5 py-0.5 text-[10px] font-medium rounded bg-amber-50 dark:bg-amber-500/10 text-amber-700 dark:text-amber-300 uppercase tracking-wide">
                    {t('settings.localModel.deviceCapability.recommended')}
                  </span>
                )}
              </div>
              <span className="text-xs text-stone-500 dark:text-neutral-400">0 GB</span>
            </div>
            <div className="text-xs text-stone-500 dark:text-neutral-400 mt-1">
              {t('settings.localModel.deviceCapability.disabledDesc')}
            </div>
          </button>

          {presetsData.presets.map(preset => {
            const isCurrent = !isDisabledActive && preset.tier === presetsData.current_tier;
            const isApplying = applying === preset.tier;
            const locked = !ollamaAvailable;
            return (
              <button
                type="button"
                key={preset.tier}
                onClick={() => void handleApply(preset.tier)}
                disabled={applying !== null || locked}
                title={
                  locked ? t('settings.localModel.deviceCapability.installOllamaFirst') : undefined
                }
                className={`w-full text-left rounded-lg border p-3 transition-colors ${
                  isCurrent
                    ? 'border-primary-400 bg-primary-50 dark:bg-primary-500/10'
                    : 'border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800'
                } ${
                  locked
                    ? 'opacity-50 cursor-not-allowed hover:bg-stone-50 dark:hover:bg-neutral-800/60 dark:bg-neutral-800/60'
                    : applying !== null && !isApplying
                      ? 'opacity-60 cursor-not-allowed'
                      : 'cursor-pointer'
                }`}>
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                      {preset.label}
                    </span>
                    {isCurrent && (
                      <span className="px-1.5 py-0.5 text-[10px] font-medium rounded bg-primary-50 dark:bg-primary-500/10 text-primary-600 dark:text-primary-300 uppercase tracking-wide">
                        {t('settings.localModel.deviceCapability.active')}
                      </span>
                    )}
                    {isApplying && (
                      <span className="px-1.5 py-0.5 text-[10px] font-medium rounded bg-stone-100 dark:bg-neutral-800 text-stone-500 dark:text-neutral-400 uppercase tracking-wide">
                        {t('settings.localModel.deviceCapability.applying')}
                      </span>
                    )}
                    {locked && (
                      <span className="px-1.5 py-0.5 text-[10px] font-medium rounded bg-amber-50 dark:bg-amber-500/10 text-amber-700 dark:text-amber-300 uppercase tracking-wide">
                        {t('settings.localModel.deviceCapability.needsOllama')}
                      </span>
                    )}
                  </div>
                  <span className="text-xs text-stone-500 dark:text-neutral-400">
                    ~{Number(preset.approx_download_gb).toFixed(1)} GB
                  </span>
                </div>
                <div className="text-xs text-stone-400 dark:text-neutral-500 mt-1">
                  {preset.description}
                </div>
                <div className="text-[10px] text-stone-500 dark:text-neutral-400 mt-1">
                  {t('settings.localModel.deviceCapability.presetDetails')
                    .replace('{chatModel}', preset.chat_model_id)
                    .replace(
                      '{visionModel}',
                      preset.vision_mode === 'disabled'
                        ? t('settings.localModel.deviceCapability.disabledLowercase')
                        : preset.vision_model_id || preset.vision_mode
                    )
                    .replace('{targetRamGb}', String(preset.target_ram_gb))}
                </div>
              </button>
            );
          })}

          {presetsData.current_tier === 'custom' && !isDisabledActive && (
            <div className="rounded-lg border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 p-3 text-xs text-amber-700 dark:text-amber-300">
              {t('settings.localModel.deviceCapability.customModelIds')}
            </div>
          )}
        </div>
      )}

      {applyError && <div className="text-xs text-red-600 dark:text-red-300">{applyError}</div>}
      {presetError && !(!presetsLoading && !presetsData) && (
        <div className="text-xs text-red-600 dark:text-red-300">{presetError}</div>
      )}
      {(applySuccess ?? presetSuccess) && (
        <div className="text-xs text-green-700 dark:text-green-300">
          {(applySuccess ?? presetSuccess)?.applied_tier === DISABLED_TIER_ID
            ? t('settings.localModel.deviceCapability.localAiDisabled')
            : t('settings.localModel.deviceCapability.appliedTier')
                .replace('{tier}', (applySuccess ?? presetSuccess)?.applied_tier ?? '')
                .replace(
                  '{model}',
                  (applySuccess ?? presetSuccess)?.chat_model_id
                    ? `: ${(applySuccess ?? presetSuccess)?.chat_model_id}`
                    : ''
                )}
        </div>
      )}
    </section>
  );
};

export default DeviceCapabilitySection;
