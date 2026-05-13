import { useState } from 'react';

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
   * When `false`, the Ollama runtime isn't installed yet. Local tiers
   * require Ollama, so they're rendered disabled with a notice that
   * lets the user install Ollama in place. The "Disabled (cloud
   * fallback)" option stays enabled since it doesn't need Ollama.
   */
  ollamaAvailable?: boolean;
  /**
   * Triggers the same install pipeline the Runtime Status section uses.
   * Wired only when `ollamaAvailable === false` to surface an inline
   * Install Ollama button next to the locked tiers.
   */
  onTriggerOllamaInstall?: () => void;
  /** True while an install pipeline is already running. */
  isTriggeringInstall?: boolean;
  /**
   * Live state from `local_ai_status` so the notice can show real install
   * progress: `installing`, `downloading`, `degraded`, etc. The button's
   * own `isTriggeringInstall` only covers the RPC round-trip (~ms);
   * `installState` covers the entire backend pipeline (~60s).
   */
  installState?: string;
  /** Latest `status.warning` text — shown under the progress label. */
  installWarning?: string | null;
  /** Latest `status.error_detail` — shown when state is `degraded`. */
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
  const installInProgress =
    installState === 'installing' || installState === 'downloading' || installState === 'loading';
  const installFailed = installState === 'degraded';
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
      const msg = err instanceof Error ? err.message : 'Failed to apply preset';
      setApplyError(msg);
    } finally {
      setApplying(null);
    }
  };

  return (
    <section className="space-y-3">
      <h3 className="text-sm font-semibold text-stone-900">Model Tier</h3>

      {presetsLoading && !presetsData && (
        <div className="bg-stone-50 rounded-lg border border-stone-200 p-4 text-sm text-stone-500 animate-pulse">
          Loading device info and presets…
        </div>
      )}
      {!presetsLoading && !presetsData && presetError && (
        <div className="bg-red-50 rounded-lg border border-red-300 p-4 text-sm text-red-600">
          Could not load presets: {presetError}
        </div>
      )}

      {presetsData?.device && (
        <div className="bg-stone-50 rounded-lg border border-stone-200 p-3">
          <div className="grid grid-cols-3 gap-3 text-xs">
            <div>
              <div className="text-stone-500 uppercase tracking-wide">RAM</div>
              <div className="text-stone-800 mt-0.5 font-medium">
                {formatRamGb(presetsData.device.total_ram_bytes)}
              </div>
            </div>
            <div>
              <div className="text-stone-500 uppercase tracking-wide">CPU</div>
              <div
                className="text-stone-800 mt-0.5 font-medium truncate"
                title={presetsData.device.cpu_brand}>
                {presetsData.device.cpu_count} cores
              </div>
            </div>
            <div>
              <div className="text-stone-500 uppercase tracking-wide">GPU</div>
              <div
                className="text-stone-800 mt-0.5 font-medium truncate"
                title={presetsData.device.gpu_description ?? undefined}>
                {presetsData.device.has_gpu
                  ? (presetsData.device.gpu_description ?? 'Detected')
                  : 'Not detected'}
              </div>
            </div>
          </div>
        </div>
      )}

      {presetsData && !ollamaAvailable && (
        <div
          className={`rounded-lg border p-3 space-y-2 ${
            installFailed
              ? 'border-red-300 bg-red-50'
              : installInProgress
                ? 'border-blue-300 bg-blue-50'
                : 'border-amber-300 bg-amber-50'
          }`}>
          {installInProgress ? (
            <>
              <div className="flex items-center gap-2">
                <div className="h-3 w-3 rounded-full border-2 border-blue-500 border-t-transparent animate-spin" />
                <div className="text-sm font-semibold text-blue-900">
                  Installing Ollama
                  {installState === 'downloading' ? ' (downloading models)' : '…'}
                </div>
              </div>
              <div className="text-xs text-blue-800">
                {installWarning ??
                  'Downloading the OllamaSetup installer (~2 GB) and unpacking it. This can take a minute on first install.'}
              </div>
              <div className="h-1.5 rounded-full bg-blue-200 overflow-hidden">
                <div className="h-full w-1/3 bg-blue-500 animate-pulse" />
              </div>
            </>
          ) : installFailed ? (
            <>
              <div className="text-sm font-semibold text-red-900">Ollama install failed</div>
              <div className="text-xs text-red-800">
                {installWarning ??
                  'The installer exited before Ollama was usable. Click retry to try again, or install manually from ollama.com.'}
              </div>
              {installError && (
                <pre className="max-h-40 overflow-auto rounded bg-red-100 border border-red-200 p-2 text-[10px] text-red-700 leading-tight whitespace-pre-wrap break-words">
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
                    {isTriggeringInstall ? 'Retrying…' : 'Retry install'}
                  </button>
                )}
                <a
                  href="https://ollama.com"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="px-3 py-1.5 text-xs rounded-md border border-red-300 hover:border-red-400 text-red-800">
                  Install manually
                </a>
              </div>
            </>
          ) : (
            <>
              <div className="text-xs text-amber-800">
                <span className="font-semibold text-amber-900">Install Ollama first.</span> Local
                tiers run on the Ollama runtime, which isn&apos;t installed yet. The &ldquo;Disabled
                (cloud fallback)&rdquo; option stays available either way.
              </div>
              <div className="flex items-center gap-2">
                {onTriggerOllamaInstall && (
                  <button
                    type="button"
                    onClick={onTriggerOllamaInstall}
                    disabled={isTriggeringInstall}
                    className="px-3 py-1.5 text-xs rounded-md bg-amber-600 hover:bg-amber-700 disabled:opacity-60 text-white font-medium">
                    {isTriggeringInstall ? 'Starting…' : 'Install Ollama'}
                  </button>
                )}
                <a
                  href="https://ollama.com"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="px-3 py-1.5 text-xs rounded-md border border-amber-300 hover:border-amber-400 text-amber-800">
                  Install manually
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
                ? 'border-primary-400 bg-primary-50'
                : 'border-stone-200 bg-stone-50 hover:bg-stone-100'
            } ${applying !== null ? 'opacity-60 cursor-not-allowed' : 'cursor-pointer'}`}>
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <span className="text-sm font-semibold text-stone-900">Disabled</span>
                {isDisabledActive && (
                  <span className="px-1.5 py-0.5 text-[10px] font-medium rounded bg-primary-50 text-primary-600 uppercase tracking-wide">
                    Active
                  </span>
                )}
                {(presetsData.recommend_disabled || !ollamaAvailable) && !isDisabledActive && (
                  <span className="px-1.5 py-0.5 text-[10px] font-medium rounded bg-amber-50 text-amber-700 uppercase tracking-wide">
                    Recommended
                  </span>
                )}
              </div>
              <span className="text-xs text-stone-500">0 GB</span>
            </div>
            <div className="text-xs text-stone-500 mt-1">
              Fallback to the cloud summarizer model. No local download or Ollama install required.
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
                title={locked ? 'Install Ollama first to use this tier' : undefined}
                className={`w-full text-left rounded-lg border p-3 transition-colors ${
                  isCurrent
                    ? 'border-primary-400 bg-primary-50'
                    : 'border-stone-200 bg-stone-50 hover:bg-stone-100'
                } ${
                  locked
                    ? 'opacity-50 cursor-not-allowed hover:bg-stone-50'
                    : applying !== null && !isApplying
                      ? 'opacity-60 cursor-not-allowed'
                      : 'cursor-pointer'
                }`}>
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-semibold text-stone-900">{preset.label}</span>
                    {isCurrent && (
                      <span className="px-1.5 py-0.5 text-[10px] font-medium rounded bg-primary-50 text-primary-600 uppercase tracking-wide">
                        Active
                      </span>
                    )}
                    {isApplying && (
                      <span className="px-1.5 py-0.5 text-[10px] font-medium rounded bg-stone-100 text-stone-500 uppercase tracking-wide">
                        Applying…
                      </span>
                    )}
                    {locked && (
                      <span className="px-1.5 py-0.5 text-[10px] font-medium rounded bg-amber-50 text-amber-700 uppercase tracking-wide">
                        Needs Ollama
                      </span>
                    )}
                  </div>
                  <span className="text-xs text-stone-500">
                    ~{Number(preset.approx_download_gb).toFixed(1)} GB
                  </span>
                </div>
                <div className="text-xs text-stone-400 mt-1">{preset.description}</div>
                <div className="text-[10px] text-stone-500 mt-1">
                  Chat: {preset.chat_model_id} &middot; Vision:{' '}
                  {preset.vision_mode === 'disabled'
                    ? 'disabled'
                    : preset.vision_model_id || preset.vision_mode}{' '}
                  &middot; Target RAM: {preset.target_ram_gb} GB
                </div>
              </button>
            );
          })}

          {presetsData.current_tier === 'custom' && !isDisabledActive && (
            <div className="rounded-lg border border-amber-200 bg-amber-50 p-3 text-xs text-amber-700">
              You are using custom model IDs that do not match any built-in preset.
            </div>
          )}
        </div>
      )}

      {applyError && <div className="text-xs text-red-600">{applyError}</div>}
      {presetError && !(!presetsLoading && !presetsData) && (
        <div className="text-xs text-red-600">{presetError}</div>
      )}
      {(applySuccess ?? presetSuccess) && (
        <div className="text-xs text-green-700">
          {(applySuccess ?? presetSuccess)?.applied_tier === DISABLED_TIER_ID
            ? 'Local AI disabled — using cloud fallback.'
            : `Applied ${(applySuccess ?? presetSuccess)?.applied_tier} tier${
                (applySuccess ?? presetSuccess)?.chat_model_id
                  ? `: ${(applySuccess ?? presetSuccess)?.chat_model_id}`
                  : ''
              }`}
        </div>
      )}
    </section>
  );
};

export default DeviceCapabilitySection;
