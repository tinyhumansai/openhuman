import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
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
  const { t } = useT();
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
      const message =
        err instanceof Error ? err.message : t('voice.debug.failedToLoadVoiceDebugData');
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
        always_on_enabled: settings.always_on_enabled,
      });
      setNotice(t('voice.debug.settingsSaved'));
      await loadData(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : t('voice.debug.failedToSaveSettings');
      setError(message);
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div>
      <SettingsHeader
        title={t('voice.debugTitle')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        {/* Runtime status section */}
        <section className="space-y-3">
          <div className="bg-stone-50 dark:bg-neutral-800/60 rounded-lg border border-stone-200 dark:border-neutral-800 p-4 space-y-3">
            <div className="flex items-center justify-between">
              <div>
                <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                  {t('voice.debug.runtimeStatus')}
                </h3>
                <p className="text-xs text-stone-500 dark:text-neutral-400 mt-1">
                  {t('voice.debug.runtimeStatusDesc')}
                </p>
              </div>
              <button
                type="button"
                onClick={() => void loadData()}
                className="text-xs text-primary-600 dark:text-primary-300 hover:text-primary-700 dark:text-primary-300">
                {t('common.refresh')}
              </button>
            </div>

            <div className="grid grid-cols-2 gap-3 text-sm">
              <div className="rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-3">
                <div className="text-[10px] uppercase tracking-wide text-stone-500 dark:text-neutral-400">
                  {t('voice.debug.server')}
                </div>
                <div className="mt-1 font-medium text-stone-900 dark:text-neutral-100">
                  {serverStatus
                    ? serverStatus.state
                    : isLoading
                      ? t('common.loading')
                      : t('voice.debug.unavailable')}
                </div>
              </div>
              <div className="rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 p-3">
                <div className="text-[10px] uppercase tracking-wide text-stone-500 dark:text-neutral-400">
                  STT
                </div>
                <div className="mt-1 font-medium text-stone-900 dark:text-neutral-100">
                  {voiceStatus?.stt_available ? t('voice.debug.ready') : t('voice.debug.notReady')}
                </div>
              </div>
            </div>

            {serverStatus && (
              <div className="grid grid-cols-1 sm:grid-cols-3 gap-3 text-xs text-stone-600 dark:text-neutral-300">
                <div>
                  {t('voice.debug.hotkey')}: {serverStatus.hotkey || t('voice.debug.notAvailable')}
                </div>
                <div>
                  {t('voice.debug.mode')}: {serverStatus.activation_mode}
                </div>
                <div>
                  {t('voice.debug.transcriptions')}: {serverStatus.transcription_count}
                </div>
              </div>
            )}

            {serverStatus?.last_error && (
              <div className="rounded-md border border-red-200 dark:border-red-500/30 bg-red-50 dark:bg-red-500/10 p-3 text-xs text-red-600 dark:text-red-300">
                <div className="font-medium mb-1">{t('voice.debug.serverError')}</div>
                {serverStatus.last_error}
              </div>
            )}
          </div>
        </section>

        {/* Advanced settings section */}
        <section className="space-y-3">
          <div className="bg-stone-50 dark:bg-neutral-800/60 rounded-lg border border-stone-200 dark:border-neutral-800 p-4 space-y-4">
            <div>
              <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                {t('voice.debug.advancedSettings')}
              </h3>
              <p className="text-xs text-stone-500 dark:text-neutral-400 mt-1">
                {t('voice.debug.advancedSettingsDesc')}
              </p>
            </div>

            {settings && (
              <>
                {/* Always-on listening (Phase 2) — opt-in, privacy-sensitive. */}
                <div className="flex items-start justify-between gap-3 rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2.5">
                  <div className="min-w-0">
                    <span className="text-xs font-medium text-stone-700 dark:text-neutral-200">
                      {t('voice.debug.alwaysOn')}
                    </span>
                    <p className="text-[11px] text-stone-400 dark:text-neutral-500 mt-0.5">
                      {t('voice.debug.alwaysOnDesc')}
                    </p>
                  </div>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={settings.always_on_enabled}
                    aria-label={t('voice.debug.alwaysOn')}
                    data-testid="voice-always-on-toggle"
                    onClick={() => updateSetting('always_on_enabled', !settings.always_on_enabled)}
                    className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors ${
                      settings.always_on_enabled
                        ? 'bg-primary-500'
                        : 'bg-stone-300 dark:bg-neutral-600'
                    }`}>
                    <span
                      aria-hidden
                      className={`inline-block h-3 w-3 transform rounded-full bg-white shadow transition-transform ${
                        settings.always_on_enabled ? 'translate-x-3.5' : 'translate-x-0.5'
                      }`}
                    />
                  </button>
                </div>

                <label className="block space-y-1">
                  <span className="text-xs font-medium text-stone-600 dark:text-neutral-300">
                    {t('voice.debug.minimumRecordingSeconds')}
                  </span>
                  <input
                    type="number"
                    min="0"
                    step="0.1"
                    value={settings.min_duration_secs}
                    onChange={e => updateSetting('min_duration_secs', Number(e.target.value) || 0)}
                    className="w-full rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 focus:outline-none focus:ring-1 focus:ring-primary-400"
                  />
                </label>

                <label className="block space-y-1">
                  <span className="text-xs font-medium text-stone-600 dark:text-neutral-300">
                    {t('voice.debug.silenceThreshold')}
                  </span>
                  <p className="text-[11px] text-stone-400 dark:text-neutral-500">
                    {t('voice.debug.silenceThresholdDesc')}
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
                    className="w-full rounded-md border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 focus:outline-none focus:ring-1 focus:ring-primary-400"
                  />
                </label>
              </>
            )}

            {error && (
              <div className="rounded-md border border-red-200 dark:border-red-500/30 bg-red-50 dark:bg-red-500/10 p-3 text-xs text-red-600 dark:text-red-300">
                {error}
              </div>
            )}
            {notice && (
              <div className="rounded-md border border-emerald-200 dark:border-emerald-500/30 bg-emerald-50 dark:bg-emerald-500/10 p-3 text-xs text-emerald-700 dark:text-emerald-300">
                {notice}
              </div>
            )}

            <div className="flex gap-2">
              <button
                type="button"
                onClick={() => void saveSettings()}
                disabled={isSaving || !hasUnsavedChanges}
                className="px-3 py-1.5 text-xs rounded-md bg-primary-600 hover:bg-primary-700 disabled:opacity-60 text-white">
                {isSaving ? t('common.loading') : t('common.save')}
              </button>
            </div>
          </div>
        </section>
      </div>
    </div>
  );
};

export default VoiceDebugPanel;
