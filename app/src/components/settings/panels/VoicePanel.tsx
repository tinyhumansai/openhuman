import { useEffect, useRef, useState } from 'react';

import { formatBytes, formatEta, statusLabel } from '../../../utils/localAiHelpers';
import {
  type LocalAiAssetStatus,
  type LocalAiDownloadProgressItem,
  openhumanGetVoiceServerSettings,
  openhumanLocalAiAssetsStatus,
  openhumanLocalAiDownloadAsset,
  openhumanLocalAiDownloadsProgress,
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

const STT_DOWNLOADING_STATES = new Set(['downloading', 'installing', 'loading']);
const STT_ERROR_STATES = new Set(['error', 'failed', 'degraded']);

const VoicePanel = () => {
  const { navigateBack, navigateToSettings, breadcrumbs } = useSettingsNavigation();
  const [settings, setSettings] = useState<VoiceServerSettings | null>(null);
  const [savedSettings, setSavedSettings] = useState<VoiceServerSettings | null>(null);
  const [serverStatus, setServerStatus] = useState<VoiceServerStatus | null>(null);
  const [, setVoiceStatus] = useState<VoiceStatus | null>(null);
  const [sttReady, setSttReady] = useState(false);
  const [sttAsset, setSttAsset] = useState<LocalAiAssetStatus | null>(null);
  const [sttDownload, setSttDownload] = useState<LocalAiDownloadProgressItem | null>(null);
  const [sttDownloadError, setSttDownloadError] = useState<string | null>(null);
  const [sttDownloadRequested, setSttDownloadRequested] = useState(false);
  const [, setIsLoading] = useState(true);
  const [isSaving, setIsSaving] = useState(false);
  const [isStarting, setIsStarting] = useState(false);
  const [isStopping, setIsStopping] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [newDictWord, setNewDictWord] = useState('');
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
      const [settingsResponse, serverResponse, voiceResponse, assetsResponse, downloadsResponse] =
        await Promise.all([
          openhumanGetVoiceServerSettings(),
          openhumanVoiceServerStatus(),
          openhumanVoiceStatus(),
          openhumanLocalAiAssetsStatus(),
          openhumanLocalAiDownloadsProgress().catch(() => null),
        ]);
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
      const sttAssetRaw = assetsResponse.result.stt ?? null;
      setSttAsset(sttAssetRaw);
      const sttAssetState = sttAssetRaw?.state;
      const sttAssetOk = sttAssetState === 'ready' || sttAssetState === 'ondemand';
      if (process.env.NODE_ENV !== 'production') {
        console.debug('[VoicePanel:stt] readiness decision', {
          sttAssetState,
          sttAssetOk,
          sttAvailable: voiceResponse.stt_available,
        });
      }
      setSttReady(sttAssetOk && voiceResponse.stt_available);
      const sttDownloadRaw = downloadsResponse?.result?.stt ?? null;
      setSttDownload(sttDownloadRaw);
      // Clear transient "requested" once core reports an active download phase
      // or the asset finished downloading (state becomes ready).
      if (sttAssetOk || (sttDownloadRaw && STT_DOWNLOADING_STATES.has(sttDownloadRaw.state))) {
        setSttDownloadRequested(false);
      }
      setError(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load voice settings';
      setError(message);
    } finally {
      setIsLoading(false);
    }
  };

  const startSttDownload = async () => {
    console.debug('[voice-intel] starting in-place STT download');
    setSttDownloadError(null);
    setSttDownloadRequested(true);
    try {
      await openhumanLocalAiDownloadAsset('stt');
      await loadData(false);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to start STT model download';
      console.debug('[voice-intel] STT download failed', message);
      setSttDownloadError(message);
      setSttDownloadRequested(false);
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
      <SettingsHeader
        title="Voice Dictation"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        <section className={`space-y-3 ${disabled ? 'opacity-60' : ''}`}>
          <div className="bg-stone-50 rounded-lg border border-stone-200 p-4 space-y-4">
            <div>
              <h3 className="text-sm font-semibold text-stone-900">Voice Settings</h3>
              <p className="text-xs text-stone-500 mt-1">
                Hold the hotkey to dictate and insert text into the active field.
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

                <label className="flex items-center gap-2 text-sm text-stone-700">
                  <input
                    type="checkbox"
                    checked={settings.auto_start}
                    onChange={e => updateSetting('auto_start', e.target.checked)}
                    className="h-4 w-4 rounded border-stone-300 text-primary-600 focus:ring-primary-500"
                  />
                  Start voice server automatically with the core
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
                            updateSetting('custom_dictionary', [
                              ...settings.custom_dictionary,
                              word,
                            ]);
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
              <SttSetupBlock
                sttAsset={sttAsset}
                sttDownload={sttDownload}
                sttDownloadError={sttDownloadError}
                sttDownloadRequested={sttDownloadRequested}
                onStartDownload={() => void startSttDownload()}
                onOpenAdvanced={() => navigateToSettings('local-model')}
              />
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

        <button
          type="button"
          onClick={() => navigateToSettings('voice-debug')}
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

interface SttSetupBlockProps {
  sttAsset: LocalAiAssetStatus | null;
  sttDownload: LocalAiDownloadProgressItem | null;
  sttDownloadError: string | null;
  sttDownloadRequested: boolean;
  onStartDownload: () => void;
  onOpenAdvanced: () => void;
}

const SttSetupBlock = ({
  sttAsset,
  sttDownload,
  sttDownloadError,
  sttDownloadRequested,
  onStartDownload,
  onOpenAdvanced,
}: SttSetupBlockProps) => {
  const assetState = sttAsset?.state ?? 'missing';
  const downloadState = sttDownload?.state ?? 'idle';
  const isDownloading = STT_DOWNLOADING_STATES.has(downloadState) || sttDownloadRequested;
  const hasCoreError =
    !sttDownloadError &&
    (STT_ERROR_STATES.has(assetState) ||
      STT_ERROR_STATES.has(downloadState) ||
      !!sttDownload?.warning);
  const coreErrorMessage =
    sttDownload?.warning ??
    sttAsset?.warning ??
    (STT_ERROR_STATES.has(assetState) || STT_ERROR_STATES.has(downloadState)
      ? 'The local STT model download did not complete.'
      : null);
  const progress =
    sttDownload && typeof sttDownload.progress === 'number'
      ? Math.max(0, Math.min(1, sttDownload.progress))
      : null;
  const percent = progress != null ? Math.round(progress * 100) : null;

  // Priority: explicit frontend error > core-reported error > active progress > idle CTA.
  if (sttDownloadError || hasCoreError) {
    const message = sttDownloadError ?? coreErrorMessage ?? 'Unable to download the STT model.';
    return (
      <div
        data-testid="voice-stt-setup-error"
        className="rounded-md border border-red-200 bg-red-50 p-4 text-sm text-red-700 space-y-3">
        <div className="space-y-1">
          <div className="font-medium">STT model download didn’t complete.</div>
          <div className="text-xs text-red-600">{message}</div>
          <div className="text-xs text-red-500">
            Check your internet connection and click Retry. The download resumes where it left off.
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            onClick={onStartDownload}
            className="px-3 py-1.5 text-xs rounded-md bg-red-600 hover:bg-red-700 text-white">
            Retry download
          </button>
          <button
            type="button"
            onClick={onOpenAdvanced}
            className="px-3 py-1.5 text-xs rounded-md border border-red-200 text-red-700 hover:border-red-300">
            Advanced (Local AI)
          </button>
        </div>
      </div>
    );
  }

  if (isDownloading) {
    const downloaded = sttDownload?.downloaded_bytes ?? null;
    const total = sttDownload?.total_bytes ?? null;
    const speed = sttDownload?.speed_bps ?? null;
    const eta = sttDownload?.eta_seconds ?? null;
    return (
      <div
        data-testid="voice-stt-setup-progress"
        className="rounded-md border border-primary-200 bg-primary-50 p-4 text-sm text-primary-900 space-y-3">
        <div className="space-y-1">
          <div className="font-medium">
            {statusLabel(STT_DOWNLOADING_STATES.has(downloadState) ? downloadState : 'downloading')}
            {' the local STT model…'}
          </div>
          <div className="text-xs text-primary-700">
            You can keep this panel open — we’ll enable Voice Dictation the moment it’s ready.
          </div>
        </div>
        <div className="h-1.5 w-full rounded-full bg-primary-100 overflow-hidden">
          <div
            className={`h-full rounded-full bg-primary-500 transition-all duration-500 ${
              downloadState === 'installing' ? 'animate-pulse' : ''
            }`}
            style={{ width: downloadState === 'installing' ? '100%' : `${percent ?? 5}%` }}
          />
        </div>
        <div className="flex items-center justify-between text-[11px] text-primary-700">
          <span>
            {downloadState === 'installing'
              ? 'Installing…'
              : downloaded != null && total != null
                ? `${formatBytes(downloaded)} / ${formatBytes(total)}`
                : percent != null
                  ? `${percent}%`
                  : 'Preparing…'}
          </span>
          <span>
            {speed != null && speed > 0 ? `${formatBytes(speed)}/s` : ''}
            {eta != null && eta > 0 ? ` · ${formatEta(eta)}` : ''}
          </span>
        </div>
      </div>
    );
  }

  return (
    <div
      data-testid="voice-stt-setup-idle"
      className="rounded-md border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800 space-y-3">
      <div className="space-y-1">
        <div className="font-medium">Voice Dictation needs the local STT model.</div>
        <div className="text-xs text-amber-700">
          It’s a one-time download (~50–150 MB depending on your device). Everything stays on your
          Mac.
        </div>
      </div>
      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          onClick={onStartDownload}
          className="px-3 py-1.5 text-xs rounded-md bg-amber-600 hover:bg-amber-700 text-white">
          Download STT model
        </button>
        <button
          type="button"
          onClick={onOpenAdvanced}
          className="px-3 py-1.5 text-xs rounded-md border border-amber-300 text-amber-800 hover:border-amber-400">
          Advanced (Local AI)
        </button>
      </div>
    </div>
  );
};

export default VoicePanel;
