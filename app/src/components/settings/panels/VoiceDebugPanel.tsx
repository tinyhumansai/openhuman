import { useEffect, useRef, useState } from 'react';

import {
  openhumanGetVoiceServerSettings,
  openhumanUpdateVoiceServerSettings,
  openhumanVoiceServerStatus,
  openhumanVoiceStatus,
  type VoiceServerSettings,
  type VoiceServerStatus,
  type VoiceStatus,
} from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const VoiceDebugPanel = () => {
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const [settings, setSettings] = useState<VoiceServerSettings | null>(null);
  const [savedSettings, setSavedSettings] = useState<VoiceServerSettings | null>(null);
  const [serverStatus, setServerStatus] = useState<VoiceServerStatus | null>(null);
  const [voiceStatus, setVoiceStatus] = useState<VoiceStatus | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const settingsRef = useRef<VoiceServerSettings | null>(null);
  const savedSettingsRef = useRef<VoiceServerSettings | null>(null);

  const hasUnsavedChanges =
    settings != null &&
    savedSettings != null &&
    JSON.stringify(settings) !== JSON.stringify(savedSettings);

  useEffect(() => {
    settingsRef.current = settings;
  }, [settings]);

  useEffect(() => {
    savedSettingsRef.current = savedSettings;
  }, [savedSettings]);

  const loadData = async (forceSettings = false) => {
    try {
      const [settingsResponse, serverResponse, voiceResponse] = await Promise.all([
        openhumanGetVoiceServerSettings(),
        openhumanVoiceServerStatus(),
        openhumanVoiceStatus(),
      ]);
      // Only overwrite local settings if there are no unsaved edits,
      // or if explicitly forced (e.g. after save or initial load).
      // This prevents the 2s polling timer from clobbering user input.
      const currentSettings = settingsRef.current;
      const currentSavedSettings = savedSettingsRef.current;
      if (
        forceSettings ||
        !currentSettings ||
        JSON.stringify(currentSettings) === JSON.stringify(currentSavedSettings)
      ) {
        setSettings(settingsResponse.result);
      }
      setSavedSettings(settingsResponse.result);
      setServerStatus(serverResponse);
      setVoiceStatus(voiceResponse);
      setError(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load voice debug data';
      setError(message);
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    void loadData(true);
    const timer = window.setInterval(() => {
      void loadData(false);
    }, 2000);
    return () => window.clearInterval(timer);
  }, []);

  const updateSetting = <K extends keyof VoiceServerSettings>(
    key: K,
    value: VoiceServerSettings[K]
  ) => {
    setSettings(current => (current ? { ...current, [key]: value } : current));
  };

  const saveSettings = async () => {
    if (!settings) return;

    setIsSaving(true);
    setError(null);
    setNotice(null);
    try {
      await openhumanUpdateVoiceServerSettings({
        auto_start: settings.auto_start,
        hotkey: settings.hotkey,
        activation_mode: settings.activation_mode,
        skip_cleanup: settings.skip_cleanup,
        min_duration_secs: settings.min_duration_secs,
        silence_threshold: settings.silence_threshold,
        custom_dictionary: settings.custom_dictionary,
      });
      setNotice('Debug settings saved.');
      await loadData(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to save voice settings';
      setError(message);
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div>
      <SettingsHeader
        title="Voice Debug"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        {/* Runtime status section */}
        <section className="space-y-3">
          <div className="bg-stone-50 rounded-lg border border-stone-200 p-4 space-y-3">
            <div className="flex items-center justify-between">
              <div>
                <h3 className="text-sm font-semibold text-stone-900">Runtime Status</h3>
                <p className="text-xs text-stone-500 mt-1">
                  Live diagnostics for the voice server and speech-to-text engine.
                </p>
              </div>
              <button
                type="button"
                onClick={() => void loadData()}
                className="text-xs text-primary-600 hover:text-primary-700">
                Refresh
              </button>
            </div>

            <div className="grid grid-cols-2 gap-3 text-sm">
              <div className="rounded-md border border-stone-200 bg-white p-3">
                <div className="text-[10px] uppercase tracking-wide text-stone-500">Server</div>
                <div className="mt-1 font-medium text-stone-900">
                  {serverStatus ? serverStatus.state : isLoading ? 'Loading…' : 'Unavailable'}
                </div>
              </div>
              <div className="rounded-md border border-stone-200 bg-white p-3">
                <div className="text-[10px] uppercase tracking-wide text-stone-500">STT</div>
                <div className="mt-1 font-medium text-stone-900">
                  {voiceStatus?.stt_available ? 'Ready' : 'Not ready'}
                </div>
              </div>
            </div>

            {serverStatus && (
              <div className="grid grid-cols-1 sm:grid-cols-3 gap-3 text-xs text-stone-600">
                <div>Hotkey: {serverStatus.hotkey || 'n/a'}</div>
                <div>Mode: {serverStatus.activation_mode}</div>
                <div>Transcriptions: {serverStatus.transcription_count}</div>
              </div>
            )}

            {serverStatus?.last_error && (
              <div className="rounded-md border border-red-200 bg-red-50 p-3 text-xs text-red-600">
                <div className="font-medium mb-1">Server Error</div>
                {serverStatus.last_error}
              </div>
            )}
          </div>
        </section>

        {/* Advanced settings section */}
        <section className="space-y-3">
          <div className="bg-stone-50 rounded-lg border border-stone-200 p-4 space-y-4">
            <div>
              <h3 className="text-sm font-semibold text-stone-900">Advanced Settings</h3>
              <p className="text-xs text-stone-500 mt-1">
                Low-level tuning parameters for recording and silence detection.
              </p>
            </div>

            {settings && (
              <>
                <label className="block space-y-1">
                  <span className="text-xs font-medium text-stone-600">
                    Minimum Recording Seconds
                  </span>
                  <input
                    type="number"
                    min="0"
                    step="0.1"
                    value={settings.min_duration_secs}
                    onChange={e => updateSetting('min_duration_secs', Number(e.target.value) || 0)}
                    className="w-full rounded-md border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 focus:outline-none focus:ring-1 focus:ring-primary-400"
                  />
                </label>

                <label className="block space-y-1">
                  <span className="text-xs font-medium text-stone-600">
                    Silence Threshold (RMS)
                  </span>
                  <p className="text-[11px] text-stone-400">
                    Recordings with energy below this are treated as silence and skipped. Lower =
                    more sensitive.
                  </p>
                  <input
                    type="number"
                    min="0"
                    max="1"
                    step="0.001"
                    value={settings.silence_threshold}
                    onChange={e =>
                      updateSetting('silence_threshold', Number(e.target.value) || 0.002)
                    }
                    className="w-full rounded-md border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 focus:outline-none focus:ring-1 focus:ring-primary-400"
                  />
                </label>
              </>
            )}

            {error && (
              <div className="rounded-md border border-red-200 bg-red-50 p-3 text-xs text-red-600">
                {error}
              </div>
            )}
            {notice && (
              <div className="rounded-md border border-emerald-200 bg-emerald-50 p-3 text-xs text-emerald-700">
                {notice}
              </div>
            )}

            <div className="flex gap-2">
              <button
                type="button"
                onClick={() => void saveSettings()}
                disabled={isSaving || !hasUnsavedChanges}
                className="px-3 py-1.5 text-xs rounded-md bg-primary-600 hover:bg-primary-700 disabled:opacity-60 text-white">
                {isSaving ? 'Saving…' : 'Save'}
              </button>
            </div>
          </div>
        </section>
      </div>
    </div>
  );
};

export default VoiceDebugPanel;
