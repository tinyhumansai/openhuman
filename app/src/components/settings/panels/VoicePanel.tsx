import { useEffect, useState } from 'react';

import {
  openhumanGetVoiceServerSettings,
  openhumanLocalAiAssetsStatus,
  openhumanUpdateVoiceServerSettings,
  openhumanVoiceServerStart,
  openhumanVoiceServerStatus,
  openhumanVoiceServerStop,
  openhumanVoiceStatus,
  type VoiceServerSettings,
  type VoiceServerStatus,
  type VoiceStatus,
} from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const VoicePanel = () => {
  const { navigateBack, navigateToSettings } = useSettingsNavigation();
  const [settings, setSettings] = useState<VoiceServerSettings | null>(null);
  const [savedSettings, setSavedSettings] = useState<VoiceServerSettings | null>(null);
  const [serverStatus, setServerStatus] = useState<VoiceServerStatus | null>(null);
  const [voiceStatus, setVoiceStatus] = useState<VoiceStatus | null>(null);
  const [sttReady, setSttReady] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [isSaving, setIsSaving] = useState(false);
  const [isStarting, setIsStarting] = useState(false);
  const [isStopping, setIsStopping] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [newDictWord, setNewDictWord] = useState('');

  const hasUnsavedChanges =
    settings != null && savedSettings != null && JSON.stringify(settings) !== JSON.stringify(savedSettings);

  const loadData = async (forceSettings = false) => {
    try {
      const [settingsResponse, serverResponse, voiceResponse, assetsResponse] = await Promise.all([
        openhumanGetVoiceServerSettings(),
        openhumanVoiceServerStatus(),
        openhumanVoiceStatus(),
        openhumanLocalAiAssetsStatus(),
      ]);
      // Only overwrite local settings if there are no unsaved edits,
      // or if explicitly forced (e.g. after save or initial load).
      // This prevents the 2s polling timer from clobbering user input.
      if (forceSettings || !settings || JSON.stringify(settings) === JSON.stringify(savedSettings)) {
        setSettings(settingsResponse.result);
      }
      setSavedSettings(settingsResponse.result);
      setServerStatus(serverResponse.result);
      setVoiceStatus(voiceResponse);
      setSttReady(assetsResponse.result.stt?.state === 'ready' && voiceResponse.stt_available);
      setError(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load voice settings';
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

  const updateSetting = <K extends keyof VoiceServerSettings>(key: K, value: VoiceServerSettings[K]) => {
    setSettings(current => (current ? { ...current, [key]: value } : current));
  };

  const saveSettings = async (restartIfRunning: boolean) => {
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

      if (restartIfRunning && serverStatus && serverStatus.state !== 'stopped') {
        await openhumanVoiceServerStop();
        await openhumanVoiceServerStart({
          hotkey: settings.hotkey,
          activation_mode: settings.activation_mode,
          skip_cleanup: settings.skip_cleanup,
        });
        setNotice('Voice server restarted with the new settings.');
      } else {
        setNotice('Voice settings saved.');
      }

      await loadData(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to save voice settings';
      setError(message);
    } finally {
      setIsSaving(false);
    }
  };

  const startServer = async () => {
    if (!settings) return;

    setIsStarting(true);
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
      await openhumanVoiceServerStart({
        hotkey: settings.hotkey,
        activation_mode: settings.activation_mode,
        skip_cleanup: settings.skip_cleanup,
      });
      setNotice('Voice server started.');
      await loadData(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to start voice server';
      setError(message);
    } finally {
      setIsStarting(false);
    }
  };

  const stopServer = async () => {
    setIsStopping(true);
    setError(null);
    setNotice(null);
    try {
      await openhumanVoiceServerStop();
      setNotice('Voice server stopped.');
      await loadData(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to stop voice server';
      setError(message);
    } finally {
      setIsStopping(false);
    }
  };

  const disabled = !sttReady;
  const isRunning = serverStatus != null && serverStatus.state !== 'stopped';

  return (
    <div>
      <SettingsHeader title="Voice Dictation" showBackButton={true} onBack={navigateBack} />

      <div className="p-4 space-y-4">
        <section className="space-y-3">
          <div className="bg-stone-50 rounded-lg border border-stone-200 p-4 space-y-3">
            <div className="flex items-center justify-between">
              <div>
                <h3 className="text-sm font-semibold text-stone-900">Runtime</h3>
                <p className="text-xs text-stone-500 mt-1">
                  Hold the hotkey to dictate and insert text into the active field.
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
                {serverStatus.last_error}
              </div>
            )}
          </div>
        </section>

        <section className={`space-y-3 ${disabled ? 'opacity-60' : ''}`}>
          <div className="bg-stone-50 rounded-lg border border-stone-200 p-4 space-y-4">
            <div>
              <h3 className="text-sm font-semibold text-stone-900">Voice Server Settings</h3>
              <p className="text-xs text-stone-500 mt-1">
                Configure startup behavior, hotkey handling, and transcription cleanup.
              </p>
            </div>

            {!disabled && settings && (
              <>
                <label className="block space-y-1">
                  <span className="text-xs font-medium text-stone-600">Hotkey</span>
                  <input
                    value={settings.hotkey}
                    onChange={e => updateSetting('hotkey', e.target.value)}
                    placeholder="Fn"
                    className="w-full rounded-md border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 placeholder:text-stone-400 focus:outline-none focus:ring-1 focus:ring-primary-400"
                  />
                </label>

                <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                  <label className="block space-y-1">
                    <span className="text-xs font-medium text-stone-600">Activation Mode</span>
                    <select
                      value={settings.activation_mode}
                      onChange={e =>
                        updateSetting('activation_mode', e.target.value as 'tap' | 'push')
                      }
                      className="w-full rounded-md border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 focus:outline-none focus:ring-1 focus:ring-primary-400">
                      <option value="push">Push to talk</option>
                      <option value="tap">Tap to toggle</option>
                    </select>
                  </label>

                  <label className="block space-y-1">
                    <span className="text-xs font-medium text-stone-600">Writing Style</span>
                    <select
                      value={settings.skip_cleanup ? 'verbatim' : 'natural'}
                      onChange={e => updateSetting('skip_cleanup', e.target.value === 'verbatim')}
                      className="w-full rounded-md border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 focus:outline-none focus:ring-1 focus:ring-primary-400">
                      <option value="verbatim">Verbatim transcription</option>
                      <option value="natural">Natural cleanup</option>
                    </select>
                  </label>
                </div>

                <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                  <label className="flex items-center gap-2 text-sm text-stone-700">
                    <input
                      type="checkbox"
                      checked={settings.auto_start}
                      onChange={e => updateSetting('auto_start', e.target.checked)}
                      className="h-4 w-4 rounded border-stone-300 text-primary-600 focus:ring-primary-500"
                    />
                    Start voice server automatically with the core
                  </label>

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
                </div>

                <label className="block space-y-1">
                  <span className="text-xs font-medium text-stone-600">
                    Silence Threshold (RMS)
                  </span>
                  <p className="text-[11px] text-stone-400">
                    Recordings with energy below this are treated as silence and skipped. Lower = more sensitive.
                  </p>
                  <input
                    type="number"
                    min="0"
                    max="1"
                    step="0.001"
                    value={settings.silence_threshold}
                    onChange={e => updateSetting('silence_threshold', Number(e.target.value) || 0.002)}
                    className="w-full rounded-md border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 focus:outline-none focus:ring-1 focus:ring-primary-400"
                  />
                </label>

                <div className="space-y-2">
                  <div>
                    <span className="text-xs font-medium text-stone-600">Custom Dictionary</span>
                    <p className="text-[11px] text-stone-400">
                      Add names, technical terms, and domain words to improve recognition accuracy.
                    </p>
                  </div>
                  <div className="flex gap-2">
                    <input
                      value={newDictWord}
                      onChange={e => setNewDictWord(e.target.value)}
                      onKeyDown={e => {
                        if (e.key === 'Enter' && newDictWord.trim()) {
                          e.preventDefault();
                          const word = newDictWord.trim();
                          if (!settings.custom_dictionary.includes(word)) {
                            updateSetting('custom_dictionary', [...settings.custom_dictionary, word]);
                          }
                          setNewDictWord('');
                        }
                      }}
                      placeholder="Add a word..."
                      className="flex-1 rounded-md border border-stone-200 bg-white px-3 py-1.5 text-sm text-stone-900 placeholder:text-stone-400 focus:outline-none focus:ring-1 focus:ring-primary-400"
                    />
                    <button
                      type="button"
                      onClick={() => {
                        const word = newDictWord.trim();
                        if (word && !settings.custom_dictionary.includes(word)) {
                          updateSetting('custom_dictionary', [...settings.custom_dictionary, word]);
                        }
                        setNewDictWord('');
                      }}
                      disabled={!newDictWord.trim()}
                      className="px-3 py-1.5 text-xs rounded-md bg-primary-600 hover:bg-primary-700 disabled:opacity-60 text-white">
                      Add
                    </button>
                  </div>
                  {settings.custom_dictionary.length > 0 && (
                    <div className="flex flex-wrap gap-1.5">
                      {settings.custom_dictionary.map(word => (
                        <span
                          key={word}
                          className="inline-flex items-center gap-1 rounded-full bg-stone-100 px-2.5 py-0.5 text-xs text-stone-700">
                          {word}
                          <button
                            type="button"
                            onClick={() =>
                              updateSetting(
                                'custom_dictionary',
                                settings.custom_dictionary.filter(w => w !== word)
                              )
                            }
                            className="ml-0.5 text-stone-400 hover:text-stone-700">
                            &times;
                          </button>
                        </span>
                      ))}
                    </div>
                  )}
                </div>
              </>
            )}

            {disabled && (
              <div className="rounded-md border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800 space-y-3">
                <div>
                  Voice dictation is disabled until the local STT model is downloaded and ready.
                </div>
                <button
                  type="button"
                  onClick={() => navigateToSettings('local-model')}
                  className="px-3 py-1.5 text-xs rounded-md bg-amber-600 hover:bg-amber-700 text-white">
                  Open Local AI Model
                </button>
              </div>
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

            <div className="flex flex-wrap gap-2">
              <button
                type="button"
                onClick={() => void saveSettings(true)}
                disabled={disabled || isSaving || !hasUnsavedChanges}
                className="px-3 py-1.5 text-xs rounded-md bg-primary-600 hover:bg-primary-700 disabled:opacity-60 text-white">
                {isSaving ? 'Saving…' : 'Save Voice Settings'}
              </button>
              <button
                type="button"
                onClick={() => void startServer()}
                disabled={disabled || isStarting}
                className="px-3 py-1.5 text-xs rounded-md bg-emerald-600 hover:bg-emerald-700 disabled:opacity-60 text-white">
                {isStarting ? 'Starting…' : 'Start Voice Server'}
              </button>
              <button
                type="button"
                onClick={() => void stopServer()}
                disabled={!isRunning || isStopping}
                className="px-3 py-1.5 text-xs rounded-md border border-stone-300 hover:border-stone-400 disabled:opacity-60 text-stone-700">
                {isStopping ? 'Stopping…' : 'Stop Voice Server'}
              </button>
            </div>
          </div>
        </section>
      </div>
    </div>
  );
};

export default VoicePanel;
