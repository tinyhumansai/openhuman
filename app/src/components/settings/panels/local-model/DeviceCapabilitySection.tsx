import type { ApplyPresetResult, PresetsResponse } from '../../../../utils/tauriCommands';

interface DeviceCapabilitySectionProps {
  presetsData: PresetsResponse | null;
  presetsLoading: boolean;
  presetError: string;
  presetSuccess: ApplyPresetResult | null;
  isApplyingPreset: boolean;
  onApplyPreset: (tier: string) => void;
  formatRamGb: (bytes: number) => string;
}

const DeviceCapabilitySection = ({
  presetsData,
  presetsLoading,
  presetError,
  presetSuccess,
  isApplyingPreset,
  onApplyPreset,
  formatRamGb,
}: DeviceCapabilitySectionProps) => {
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
          <div className="rounded-lg border border-primary-200 bg-primary-50 p-3 text-xs text-primary-700">
            The local AI model is fixed for the MVP release. Broader model options will be
            available in a future update.
          </div>
          {presetsData.presets.map(preset => {
            const isCurrent = preset.tier === presetsData.current_tier;
            const isLocked = !isCurrent;
            return (
              <div
                key={preset.tier}
                className={`w-full text-left rounded-lg border p-3 ${
                  isCurrent
                    ? 'border-primary-400 bg-primary-50'
                    : 'border-stone-200 bg-stone-50 opacity-50'
                }`}>
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-semibold text-stone-900">{preset.label}</span>
                    {isCurrent && (
                      <span className="px-1.5 py-0.5 text-[10px] font-medium rounded bg-primary-50 text-primary-600 uppercase tracking-wide">
                        Active
                      </span>
                    )}
                    {isLocked && (
                      <span className="px-1.5 py-0.5 text-[10px] font-medium rounded bg-stone-100 text-stone-400 uppercase tracking-wide">
                        Coming soon
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
              </div>
            );
          })}

          {presetsData.current_tier === 'custom' && (
            <div className="rounded-lg border border-amber-200 bg-amber-50 p-3 text-xs text-amber-700">
              You are using custom model IDs that do not match any built-in preset.
            </div>
          )}
        </div>
      )}

      {presetError && !(!presetsLoading && !presetsData) && (
        <div className="text-xs text-red-600">{presetError}</div>
      )}
      {presetSuccess && (
        <div className="text-xs text-green-700">
          Applied {presetSuccess.applied_tier} tier: {presetSuccess.chat_model_id}
        </div>
      )}
    </section>
  );
};

export default DeviceCapabilitySection;
