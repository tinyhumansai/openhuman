import { useEffect, useRef, useState } from 'react';

import {
  installPiper,
  installWhisper,
  piperInstallStatus,
  type VoiceInstallStatus,
  whisperInstallStatus,
} from '../../../services/api/voiceInstallApi';
import {
  openhumanGetVoiceServerSettings,
  openhumanLocalAiAssetsStatus,
  openhumanUpdateVoiceServerSettings,
  openhumanVoiceServerStart,
  openhumanVoiceServerStatus,
  openhumanVoiceServerStop,
  openhumanVoiceSetProviders,
  openhumanVoiceStatus,
  type VoiceProvidersSnapshot,
  type VoiceServerSettings,
  type VoiceServerStatus,
  type VoiceStatus,
} from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

// Curated Piper voice presets — a handful of well-known English voices
// covering male/female and US/GB accents at the recommended `medium`
// quality tier. The full catalogue at
// huggingface.co/rhasspy/piper-voices has 100+ voices; a dropdown of
// every option is unusable so we ship a starter set and keep the free-
// text input as an escape hatch via the "Other…" option.
const PIPER_VOICE_PRESETS: ReadonlyArray<{ id: string; label: string }> = [
  { id: 'en_US-lessac-medium', label: 'US · Lessac (neutral, recommended)' },
  { id: 'en_US-lessac-high', label: 'US · Lessac (higher quality, larger)' },
  { id: 'en_US-ryan-medium', label: 'US · Ryan (male)' },
  { id: 'en_US-amy-medium', label: 'US · Amy (female)' },
  { id: 'en_US-libritts-high', label: 'US · LibriTTS (multi-speaker)' },
  { id: 'en_GB-alan-medium', label: 'GB · Alan (male)' },
  { id: 'en_GB-jenny_dioco-medium', label: 'GB · Jenny Dioco (female)' },
  { id: 'en_GB-northern_english_male-medium', label: 'GB · Northern English (male)' },
];

const VoicePanel = () => {
  const { navigateBack, navigateToSettings, breadcrumbs } = useSettingsNavigation();
  const [settings, setSettings] = useState<VoiceServerSettings | null>(null);
  const [savedSettings, setSavedSettings] = useState<VoiceServerSettings | null>(null);
  const [serverStatus, setServerStatus] = useState<VoiceServerStatus | null>(null);
  const [voiceStatus, setVoiceStatus] = useState<VoiceStatus | null>(null);
  const [sttReady, setSttReady] = useState(false);
  // Local provider selectors — initialised from voice_status, persisted via
  // openhumanVoiceSetProviders on change. Empty string until first load.
  const [sttProvider, setSttProvider] = useState<'cloud' | 'whisper' | ''>('');
  const [ttsProvider, setTtsProvider] = useState<'cloud' | 'piper' | ''>('');
  const [sttModel, setSttModel] = useState<string>('');
  const [ttsVoice, setTtsVoice] = useState<string>('');
  const [isSavingProviders, setIsSavingProviders] = useState(false);
  const [whisperInstall, setWhisperInstall] = useState<VoiceInstallStatus | null>(null);
  const [piperInstall, setPiperInstall] = useState<VoiceInstallStatus | null>(null);
  const [isInstallingWhisper, setIsInstallingWhisper] = useState(false);
  const [isInstallingPiper, setIsInstallingPiper] = useState(false);
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
      const [
        settingsResponse,
        serverResponse,
        voiceResponse,
        assetsResponse,
        whisperStatusResponse,
        piperStatusResponse,
      ] = await Promise.all([
        openhumanGetVoiceServerSettings(),
        openhumanVoiceServerStatus(),
        openhumanVoiceStatus(),
        openhumanLocalAiAssetsStatus(),
        whisperInstallStatus().catch(err => {
          // Status polls happen on a 2s loop; a single transient error
          // shouldn't blow up the entire settings panel. Log + keep the
          // previous snapshot.
          if (process.env.NODE_ENV !== 'production') {
            console.debug('[voice-install:whisper] status poll failed', err);
          }
          return null;
        }),
        piperInstallStatus().catch(err => {
          if (process.env.NODE_ENV !== 'production') {
            console.debug('[voice-install:piper] status poll failed', err);
          }
          return null;
        }),
      ]);
      if (whisperStatusResponse) setWhisperInstall(whisperStatusResponse);
      if (piperStatusResponse) setPiperInstall(piperStatusResponse);
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
      // Seed provider dropdowns from core state on first load. Use the
      // functional updater form so the check reads *current* state rather
      // than the stale closure captured when the interval was created —
      // otherwise every poll tick could re-apply the server value and
      // clobber an in-flight user edit.
      if (voiceResponse.stt_provider) {
        const seeded = voiceResponse.stt_provider === 'whisper' ? 'whisper' : 'cloud';
        setSttProvider(prev => prev || seeded);
      }
      if (voiceResponse.tts_provider) {
        const seeded = voiceResponse.tts_provider === 'piper' ? 'piper' : 'cloud';
        setTtsProvider(prev => prev || seeded);
      }
      if (voiceResponse.stt_model_id) {
        setSttModel(prev => prev || voiceResponse.stt_model_id);
      }
      if (voiceResponse.tts_voice_id) {
        setTtsVoice(prev => prev || voiceResponse.tts_voice_id);
      }
      const sttAssetState = assetsResponse.result.stt?.state;
      const sttAssetOk = sttAssetState === 'ready' || sttAssetState === 'ondemand';
      if (process.env.NODE_ENV !== 'production') {
        console.debug('[VoicePanel:stt] readiness decision', {
          sttAssetState,
          sttAssetOk,
          sttAvailable: voiceResponse.stt_available,
        });
      }
      setSttReady(sttAssetOk && voiceResponse.stt_available);
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

  const persistProviders = async (
    update: Partial<VoiceProvidersSnapshot> & {
      stt_provider?: 'cloud' | 'whisper';
      tts_provider?: 'cloud' | 'piper';
      stt_model?: string;
      tts_voice?: string;
    }
  ) => {
    setIsSavingProviders(true);
    setError(null);
    try {
      const snapshot = await openhumanVoiceSetProviders({
        stt_provider: update.stt_provider,
        tts_provider: update.tts_provider,
        stt_model: update.stt_model,
        tts_voice: update.tts_voice,
      });
      if (process.env.NODE_ENV !== 'production') {
        console.debug('[VoicePanel:providers] saved', snapshot);
      }
      setNotice('Voice providers saved.');
      // Force a reload so the rest of the panel reflects the new state.
      await loadData(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to save voice providers';
      setError(message);
    } finally {
      setIsSavingProviders(false);
    }
  };

  const onSttProviderChange = (next: 'cloud' | 'whisper') => {
    setSttProvider(next);
    void persistProviders({ stt_provider: next });
  };
  const onTtsProviderChange = (next: 'cloud' | 'piper') => {
    setTtsProvider(next);
    void persistProviders({ tts_provider: next });
  };

  /**
   * Map an install status snapshot to a button label. Single source of
   * truth for the four states the UI surfaces: Not installed / Install /
   * Installing N% / Reinstall.
   */
  const installButtonLabel = (
    status: VoiceInstallStatus | null,
    busy: boolean,
    _engine: 'Whisper' | 'Piper'
  ): string => {
    // Render based on the remote status — the install RPC is fire-and-forget,
    // so the local `busy` flag only covers the brief moment between click and
    // the RPC return. The real "is install running?" signal comes from the
    // polled status table, which lags behind by at most one 2s tick.
    if (status?.state === 'installing') {
      const pct = typeof status.progress === 'number' ? `${status.progress}%` : '…';
      return `Installing ${pct}`;
    }
    if (busy) return 'Installing…';
    if (status?.state === 'installed') return 'Reinstall locally';
    if (status?.state === 'broken') return 'Repair';
    if (status?.state === 'error') return 'Retry locally';
    return 'Install locally';
  };

  const handleInstallWhisper = async () => {
    setIsInstallingWhisper(true);
    setError(null);
    setNotice(null);
    try {
      const force = whisperInstall?.state === 'installed';
      console.debug('[voice-install:whisper] install click force=%s', force);
      const result = await installWhisper({ modelSize: sttModel || undefined, force });
      setWhisperInstall(result);
      setNotice(
        result.state === 'installed'
          ? 'Whisper is ready.'
          : `Whisper install started (${result.stage ?? 'queued'})`
      );
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to install Whisper';
      setError(message);
    } finally {
      setIsInstallingWhisper(false);
      await loadData(false);
    }
  };

  const handleInstallPiper = async () => {
    setIsInstallingPiper(true);
    setError(null);
    setNotice(null);
    try {
      const force = piperInstall?.state === 'installed';
      console.debug('[voice-install:piper] install click force=%s', force);
      const result = await installPiper({ voiceId: ttsVoice || undefined, force });
      setPiperInstall(result);
      setNotice(
        result.state === 'installed'
          ? 'Piper is ready.'
          : `Piper install started (${result.stage ?? 'queued'})`
      );
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to install Piper';
      setError(message);
    } finally {
      setIsInstallingPiper(false);
      await loadData(false);
    }
  };

  const whisperReady = whisperInstall?.state === 'installed';
  const piperReady = piperInstall?.state === 'installed';

  return (
    <div>
      <SettingsHeader
        title="Voice"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-4">
        <section className="space-y-3">
          <div
            className="bg-stone-50 rounded-lg border border-stone-200 p-4 space-y-4"
            data-testid="voice-providers-section">
            <div>
              <h3 className="text-sm font-semibold text-stone-900">Voice Providers</h3>
              <p className="text-xs text-stone-500 mt-1">
                Choose where transcription and synthesis run. Use the Install locally buttons to
                download the binaries and models into your workspace — no manual{' '}
                <code>WHISPER_BIN</code> or <code>PIPER_BIN</code> setup required.
              </p>
            </div>
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
              <label className="block space-y-1">
                <span className="text-xs font-medium text-stone-600">Speech-to-Text Provider</span>
                <select
                  aria-label="STT provider"
                  data-testid="stt-provider-select"
                  value={sttProvider || 'cloud'}
                  disabled={isSavingProviders}
                  onChange={e => onSttProviderChange(e.target.value as 'cloud' | 'whisper')}
                  className="w-full rounded-md border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 focus:outline-none focus:ring-1 focus:ring-primary-400">
                  <option value="cloud">Cloud (Whisper proxy)</option>
                  <option value="whisper" disabled={!whisperReady}>
                    Local Whisper{whisperReady ? '' : ' (install required)'}
                  </option>
                </select>
                <div className="flex items-center gap-2 pt-1">
                  <button
                    type="button"
                    data-testid="install-whisper-button"
                    onClick={() => void handleInstallWhisper()}
                    disabled={isInstallingWhisper || whisperInstall?.state === 'installing'}
                    title={
                      whisperReady
                        ? 'Whisper is installed. Click to reinstall.'
                        : 'Download whisper.cpp and the GGML model into your workspace.'
                    }
                    className={`px-2.5 py-1 text-[11px] rounded-md text-white disabled:opacity-60 ${
                      whisperReady
                        ? 'bg-stone-600 hover:bg-stone-700'
                        : 'bg-primary-600 hover:bg-primary-700'
                    }`}>
                    {installButtonLabel(whisperInstall, isInstallingWhisper, 'Whisper')}
                  </button>
                  <span
                    data-testid="whisper-install-state"
                    className={`text-[11px] ${
                      whisperReady
                        ? 'text-emerald-600'
                        : whisperInstall?.state === 'error'
                          ? 'text-red-600'
                          : 'text-stone-500'
                    }`}>
                    {whisperInstall?.state === 'installing' && whisperInstall.stage
                      ? whisperInstall.stage
                      : whisperReady
                        ? 'Installed'
                        : whisperInstall?.state === 'error'
                          ? (whisperInstall.error_detail ?? 'Install failed')
                          : 'Not installed'}
                  </span>
                </div>
              </label>
              {sttProvider === 'whisper' && (
                <label className="block space-y-1">
                  <span className="text-xs font-medium text-stone-600">Whisper Model</span>
                  <select
                    aria-label="Whisper model"
                    data-testid="stt-model-select"
                    value={sttModel || 'medium'}
                    disabled={isSavingProviders}
                    onChange={e => {
                      const nextModel = e.target.value;
                      setSttModel(nextModel);
                      void persistProviders({ stt_model: nextModel });
                      // Trigger install for the newly-selected model. The
                      // RPC is fire-and-forget + idempotent: if the .bin
                      // is already on disk, install_whisper short-circuits;
                      // if missing, status polling renders the download
                      // progress in the Install button inline.
                      void installWhisper({ modelSize: nextModel }).catch(err =>
                        console.warn(
                          '[voice-install:whisper] auto-install on model change failed:',
                          err
                        )
                      );
                    }}
                    className="w-full rounded-md border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 focus:outline-none focus:ring-1 focus:ring-primary-400">
                    <option value="tiny">Tiny (39 MB, fastest)</option>
                    <option value="base">Base (74 MB)</option>
                    <option value="small">Small (244 MB)</option>
                    <option value="medium">Medium (769 MB, recommended)</option>
                    <option value="whisper-large-v3-turbo">
                      Large v3 Turbo (1.5 GB, best accuracy)
                    </option>
                  </select>
                </label>
              )}
              <label className="block space-y-1">
                <span className="text-xs font-medium text-stone-600">Text-to-Speech Provider</span>
                <select
                  aria-label="TTS provider"
                  data-testid="tts-provider-select"
                  value={ttsProvider || 'cloud'}
                  disabled={isSavingProviders}
                  onChange={e => onTtsProviderChange(e.target.value as 'cloud' | 'piper')}
                  className="w-full rounded-md border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 focus:outline-none focus:ring-1 focus:ring-primary-400">
                  <option value="cloud">Cloud (ElevenLabs proxy)</option>
                  <option value="piper" disabled={!piperReady}>
                    Local Piper{piperReady ? '' : ' (install required)'}
                  </option>
                </select>
                <div className="flex items-center gap-2 pt-1">
                  <button
                    type="button"
                    data-testid="install-piper-button"
                    onClick={() => void handleInstallPiper()}
                    disabled={isInstallingPiper || piperInstall?.state === 'installing'}
                    title={
                      piperReady
                        ? 'Piper is installed. Click to reinstall.'
                        : 'Download Piper and the bundled en_US-lessac-medium voice into your workspace.'
                    }
                    className={`px-2.5 py-1 text-[11px] rounded-md text-white disabled:opacity-60 ${
                      piperReady
                        ? 'bg-stone-600 hover:bg-stone-700'
                        : 'bg-primary-600 hover:bg-primary-700'
                    }`}>
                    {installButtonLabel(piperInstall, isInstallingPiper, 'Piper')}
                  </button>
                  <span
                    data-testid="piper-install-state"
                    className={`text-[11px] ${
                      piperReady
                        ? 'text-emerald-600'
                        : piperInstall?.state === 'error'
                          ? 'text-red-600'
                          : 'text-stone-500'
                    }`}>
                    {piperInstall?.state === 'installing' && piperInstall.stage
                      ? piperInstall.stage
                      : piperReady
                        ? 'Installed'
                        : piperInstall?.state === 'error'
                          ? (piperInstall.error_detail ?? 'Install failed')
                          : 'Not installed'}
                  </span>
                </div>
              </label>
              {ttsProvider === 'piper' && (
                <label className="block space-y-1">
                  <span className="text-xs font-medium text-stone-600">Piper Voice</span>
                  <select
                    aria-label="Piper voice"
                    data-testid="tts-voice-select"
                    value={
                      PIPER_VOICE_PRESETS.some(v => v.id === ttsVoice) ? ttsVoice : '__custom__'
                    }
                    disabled={isSavingProviders}
                    onChange={e => {
                      const next = e.target.value;
                      if (next === '__custom__') {
                        // Keep current free-text value; the text input below
                        // becomes the editor.
                        return;
                      }
                      setTtsVoice(next);
                      void persistProviders({ tts_voice: next });
                      // Auto-fetch the .onnx for the new voice if missing.
                      // install_piper is fire-and-forget; status polling
                      // shows download progress in the Install button.
                      void installPiper({ voiceId: next }).catch(err =>
                        console.warn(
                          '[voice-install:piper] auto-install on voice change failed:',
                          err
                        )
                      );
                    }}
                    className="w-full rounded-md border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 focus:outline-none focus:ring-1 focus:ring-primary-400">
                    {PIPER_VOICE_PRESETS.map(v => (
                      <option key={v.id} value={v.id}>
                        {v.label}
                      </option>
                    ))}
                    <option value="__custom__">Other (type below)…</option>
                  </select>
                  {!PIPER_VOICE_PRESETS.some(v => v.id === ttsVoice) && (
                    <input
                      aria-label="Piper voice id (custom)"
                      data-testid="tts-voice-input"
                      value={ttsVoice}
                      placeholder="en_US-lessac-medium"
                      disabled={isSavingProviders}
                      onChange={e => setTtsVoice(e.target.value)}
                      onBlur={() => {
                        if (ttsVoice && ttsVoice !== voiceStatus?.tts_voice_id) {
                          void persistProviders({ tts_voice: ttsVoice });
                          void installPiper({ voiceId: ttsVoice }).catch(err =>
                            console.warn(
                              '[voice-install:piper] auto-install on custom voice failed:',
                              err
                            )
                          );
                        }
                      }}
                      className="mt-1 w-full rounded-md border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 placeholder:text-stone-400 focus:outline-none focus:ring-1 focus:ring-primary-400"
                    />
                  )}
                  <p className="text-[11px] text-stone-500 mt-0.5">
                    Voices come from{' '}
                    <code className="font-mono">huggingface.co/rhasspy/piper-voices</code>.
                    Switching voices may require an Install/Reinstall click to download the new{' '}
                    <code>.onnx</code>.
                  </p>
                </label>
              )}
            </div>
          </div>
        </section>

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
              <div className="rounded-md border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800">
                Voice dictation is disabled until the local STT model is downloaded. Use the{' '}
                <strong>Voice Providers</strong> section above to install Whisper.
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

export default VoicePanel;
