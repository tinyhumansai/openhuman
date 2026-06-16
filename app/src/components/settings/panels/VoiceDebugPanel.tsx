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
import PanelPage from '../../layout/PanelPage';
import Button from '../../ui/Button';
import SettingsBackButton from '../components/SettingsBackButton';
import { SettingsNumberField, SettingsRow, SettingsSection, SettingsStatusLine } from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const VoiceDebugPanel = () => {
  const { t } = useT();
  const { navigateBack } = useSettingsNavigation();
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
    <PanelPage
      className="z-10"
      contentClassName=""
      description={t('settings.developerMenu.voiceDebug.desc')}
      leading={<SettingsBackButton onBack={navigateBack} />}>
      <div className="p-4 pt-2 space-y-5">
        {/* Runtime status section */}
        <SettingsSection
          title={t('voice.debug.runtimeStatus')}
          description={t('voice.debug.runtimeStatusDesc')}>
          <SettingsRow
            stacked
            control={
              <div className="space-y-3">
                <div className="flex items-center justify-end">
                  <Button type="button" variant="ghost" size="xs" onClick={() => void loadData()}>
                    {t('common.refresh')}
                  </Button>
                </div>

                <div className="grid grid-cols-2 gap-3 text-sm">
                  <div className="rounded-md border border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-800/60 p-3">
                    <div className="text-[10px] uppercase tracking-wide text-neutral-500 dark:text-neutral-400">
                      {t('voice.debug.server')}
                    </div>
                    <div className="mt-1 font-medium text-neutral-800 dark:text-neutral-100">
                      {serverStatus
                        ? serverStatus.state
                        : isLoading
                          ? t('common.loading')
                          : t('voice.debug.unavailable')}
                    </div>
                  </div>
                  <div className="rounded-md border border-neutral-200 dark:border-neutral-800 bg-neutral-50 dark:bg-neutral-800/60 p-3">
                    <div className="text-[10px] uppercase tracking-wide text-neutral-500 dark:text-neutral-400">
                      STT
                    </div>
                    <div className="mt-1 font-medium text-neutral-800 dark:text-neutral-100">
                      {voiceStatus?.stt_available
                        ? t('voice.debug.ready')
                        : t('voice.debug.notReady')}
                    </div>
                  </div>
                </div>

                {serverStatus && (
                  <div className="grid grid-cols-1 sm:grid-cols-3 gap-3 text-xs text-neutral-500 dark:text-neutral-400">
                    <div>
                      {t('voice.debug.hotkey')}:{' '}
                      {serverStatus.hotkey || t('voice.debug.notAvailable')}
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
            }
          />
        </SettingsSection>

        {/* Advanced settings section */}
        <SettingsSection
          title={t('voice.debug.advancedSettings')}
          description={t('voice.debug.advancedSettingsDesc')}>
          {settings && (
            <>
              {/* Always-on listening moved to Settings → Features → Desktop Agent. */}
              <SettingsRow
                stacked
                label={t('voice.debug.minimumRecordingSeconds')}
                control={
                  <SettingsNumberField
                    id="min-duration-input"
                    value={String(settings.min_duration_secs)}
                    onChange={val => updateSetting('min_duration_secs', Number(val) || 0)}
                    onCommit={() => {}}
                    min={0}
                    aria-label={t('voice.debug.minimumRecordingSeconds')}
                  />
                }
              />
              <SettingsRow
                stacked
                label={t('voice.debug.silenceThreshold')}
                description={t('voice.debug.silenceThresholdDesc')}
                control={
                  <SettingsNumberField
                    id="silence-threshold-input"
                    value={String(settings.silence_threshold)}
                    onChange={val => updateSetting('silence_threshold', Number(val) || 0.002)}
                    onCommit={() => {}}
                    min={0}
                    max={1}
                    step={0.001}
                    aria-label={t('voice.debug.silenceThreshold')}
                  />
                }
              />
            </>
          )}
          <div className="px-4 py-3 space-y-3">
            <SettingsStatusLine
              saving={isSaving}
              savedNote={notice}
              error={error}
              savingLabel={t('common.loading')}
            />
            <Button
              type="button"
              variant="primary"
              size="sm"
              onClick={() => void saveSettings()}
              disabled={isSaving || !hasUnsavedChanges}>
              {isSaving ? t('common.loading') : t('common.save')}
            </Button>
          </div>
        </SettingsSection>
      </div>
    </PanelPage>
  );
};

export default VoiceDebugPanel;
