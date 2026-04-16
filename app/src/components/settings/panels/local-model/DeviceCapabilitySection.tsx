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
}

const DISABLED_TIER_ID = 'disabled';

const DeviceCapabilitySection = ({
  presetsData,
  presetsLoading,
  presetError,
  presetSuccess,
  formatRamGb,
  onPresetApplied,
}: DeviceCapabilitySectionProps) => {
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
                {presetsData.recommend_disabled && !isDisabledActive && (
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
            return (
              <button
                type="button"
                key={preset.tier}
                onClick={() => void handleApply(preset.tier)}
                disabled={applying !== null}
                className={`w-full text-left rounded-lg border p-3 transition-colors ${
                  isCurrent
                    ? 'border-primary-400 bg-primary-50'
                    : 'border-stone-200 bg-stone-50 hover:bg-stone-100'
                } ${applying !== null && !isApplying ? 'opacity-60 cursor-not-allowed' : 'cursor-pointer'}`}>
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
