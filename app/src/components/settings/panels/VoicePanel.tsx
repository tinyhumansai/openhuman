import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import PttSettingsPanel from '../../../pages/settings/voice/PttSettingsPanel';
import {
  installPiper,
  installWhisper,
  piperInstallStatus,
  type VoiceInstallStatus,
  whisperInstallStatus,
} from '../../../services/api/voiceInstallApi';
import {
  clearVoiceProviderKey,
  loadVoiceSettings,
  saveVoiceSettings,
  setVoiceProviderKey,
  testVoiceProvider,
  type VoiceProviderView,
  type VoiceSettings,
} from '../../../services/api/voiceSettingsApi';
import {
  openhumanGetVoiceServerSettings,
  openhumanVoiceSetProviders,
  openhumanVoiceStatus,
  type VoiceProvidersSnapshot,
  type VoiceServerSettings,
  type VoiceStatus,
} from '../../../utils/tauriCommands';
import PanelPage from '../../layout/PanelPage';
import Button from '../../ui/Button';
import SettingsBackButton from '../components/SettingsBackButton';
import {
  SettingsRow,
  SettingsSection,
  SettingsSelect,
  SettingsStatusLine,
  SettingsTextField,
} from '../controls';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import { ELEVENLABS_VOICE_PRESETS, isCuratedVoicePreset } from './elevenlabsVoicePresets';

/** Built-in voice provider slugs with display metadata. */
const BUILTIN_VOICE_PROVIDER_META: Record<
  string,
  { label: string; tone: string; capability: 'stt' | 'tts' | 'both'; comingSoon?: boolean }
> = {
  deepgram: {
    label: 'Deepgram',
    tone: 'bg-blue-50 text-blue-700 ring-blue-200 dark:bg-blue-900/30 dark:text-blue-300 dark:ring-blue-700',
    capability: 'stt',
    comingSoon: true,
  },
  elevenlabs: {
    label: 'ElevenLabs',
    tone: 'bg-purple-50 text-purple-700 ring-purple-200 dark:bg-purple-900/30 dark:text-purple-300 dark:ring-purple-700',
    capability: 'both',
  },
  openai: {
    label: 'OpenAI',
    tone: 'bg-emerald-50 text-emerald-700 ring-emerald-200 dark:bg-emerald-900/30 dark:text-emerald-300 dark:ring-emerald-700',
    capability: 'both',
    comingSoon: true,
  },
};

/** Local provider (Whisper/Piper) chip tone — no API key required. */
const LOCAL_VOICE_PROVIDER_TONE: Record<'whisper' | 'piper', string> = {
  whisper:
    'bg-amber-50 text-amber-700 ring-amber-200 dark:bg-amber-900/30 dark:text-amber-300 dark:ring-amber-700',
  piper:
    'bg-teal-50 text-teal-700 ring-teal-200 dark:bg-teal-900/30 dark:text-teal-300 dark:ring-teal-700',
};

// Curated Piper voice presets — a handful of well-known English voices
// covering male/female and US/GB accents at the recommended `medium`
// quality tier. The full catalogue at
// huggingface.co/rhasspy/piper-voices has 100+ voices; a dropdown of
// every option is unusable so we ship a starter set and keep the free-
// text input as an escape hatch via the "Other…" option.
const PIPER_VOICE_PRESET_IDS = [
  'en_US-lessac-medium',
  'en_US-lessac-high',
  'en_US-ryan-medium',
  'en_US-amy-medium',
  'en_US-libritts-high',
  'en_GB-alan-medium',
  'en_GB-jenny_dioco-medium',
  'en_GB-northern_english_male-medium',
] as const;

const LOCAL_INSTALL_STATUS_POLL_MS = 2_000;
const log = debug('voice:settings');

interface VoicePanelProps {
  /** When true, render without the SettingsHeader chrome (used when embedded
   *  inside the onboarding custom wizard). */
  embedded?: boolean;
}

const VoicePanel = ({ embedded = false }: VoicePanelProps = {}) => {
  const { t } = useT();
  const { navigateBack, navigateToSettings } = useSettingsNavigation();
  const [settings, setSettings] = useState<VoiceServerSettings | null>(null);
  const [savedSettings, setSavedSettings] = useState<VoiceServerSettings | null>(null);
  const [voiceStatus, setVoiceStatus] = useState<VoiceStatus | null>(null);
  // Local provider selectors — initialised from voice_status, persisted via
  // openhumanVoiceSetProviders on change. Empty string until first load.
  const [sttProvider, setSttProvider] = useState<string>('');
  const [ttsProvider, setTtsProvider] = useState<string>('');
  const [savedSttProvider, setSavedSttProvider] = useState<string>('');
  const [savedTtsProvider, setSavedTtsProvider] = useState<string>('');
  const [isSavingRouting, setIsSavingRouting] = useState(false);
  const [sttModel, setSttModel] = useState<string>('');
  const [ttsVoice, setTtsVoice] = useState<string>('');
  const [elevenlabsVoiceId, setElevenlabsVoiceId] = useState<string>('JBFqnCBsd6RMkjVDRZzb');
  const [isSavingProviders, setIsSavingProviders] = useState(false);
  const [whisperInstall, setWhisperInstall] = useState<VoiceInstallStatus | null>(null);
  const [piperInstall, setPiperInstall] = useState<VoiceInstallStatus | null>(null);
  const [isInstallingWhisper, setIsInstallingWhisper] = useState(false);
  const [isInstallingPiper, setIsInstallingPiper] = useState(false);
  const [, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  // Voice provider registry state
  const [voiceSettings, setVoiceSettings] = useState<VoiceSettings | null>(null);
  // Chip-toggle inline API-key form state
  const [pendingKeySlug, setPendingKeySlug] = useState<string | null>(null);
  const [pendingKeyValue, setPendingKeyValue] = useState('');
  const [isSavingPendingKey, setIsSavingPendingKey] = useState(false);
  const [isTestingKey, setIsTestingKey] = useState(false);
  const [keyTestResult, setKeyTestResult] = useState<{ ok: boolean; detail: string } | null>(null);
  const [isTestingStt, setIsTestingStt] = useState(false);
  const [sttTestResult, setSttTestResult] = useState<{ ok: boolean; detail: string } | null>(null);
  const [isTestingTts, setIsTestingTts] = useState(false);
  const [ttsTestResult, setTtsTestResult] = useState<{ ok: boolean; detail: string } | null>(null);
  const settingsRef = useRef<VoiceServerSettings | null>(null);
  const savedSettingsRef = useRef<VoiceServerSettings | null>(null);
  const piperVoicePresets: ReadonlyArray<{ id: string; label: string }> = [
    { id: 'en_US-lessac-medium', label: t('voice.providers.piperPreset.lessacMedium') },
    { id: 'en_US-lessac-high', label: t('voice.providers.piperPreset.lessacHigh') },
    { id: 'en_US-ryan-medium', label: t('voice.providers.piperPreset.ryanMedium') },
    { id: 'en_US-amy-medium', label: t('voice.providers.piperPreset.amyMedium') },
    { id: 'en_US-libritts-high', label: t('voice.providers.piperPreset.librittsHigh') },
    { id: 'en_GB-alan-medium', label: t('voice.providers.piperPreset.alanMedium') },
    { id: 'en_GB-jenny_dioco-medium', label: t('voice.providers.piperPreset.jennyDiocoMedium') },
    {
      id: 'en_GB-northern_english_male-medium',
      label: t('voice.providers.piperPreset.northernEnglishMaleMedium'),
    },
  ];

  useEffect(() => {
    settingsRef.current = settings;
  }, [settings]);

  useEffect(() => {
    savedSettingsRef.current = savedSettings;
  }, [savedSettings]);

  const loadData = async (forceSettings = false) => {
    try {
      const [settingsResponse, voiceResponse, whisperStatusResponse, piperStatusResponse] =
        await Promise.all([
          openhumanGetVoiceServerSettings(),
          openhumanVoiceStatus(),
          whisperInstallStatus().catch(err => {
            // Status polls happen on a 2s loop; a single transient error
            // shouldn't blow up the entire settings panel. Log + keep the
            // previous snapshot.
            log('[voice-install:whisper] status poll failed %o', err);
            return null;
          }),
          piperInstallStatus().catch(err => {
            log('[voice-install:piper] status poll failed %o', err);
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
      setVoiceStatus(voiceResponse);
      // Seed model/voice IDs from voice_status on first load only.
      if (voiceResponse.stt_model_id) {
        setSttModel(prev => prev || voiceResponse.stt_model_id);
      }
      if (voiceResponse.tts_voice_id) {
        setTtsVoice(prev => prev || voiceResponse.tts_voice_id);
      }
      // Load voice provider registry settings. This is the authoritative
      // source for stt_provider / tts_provider routing — NOT voice_status
      // (which reads from the legacy local_ai fields and doesn't know
      // about external providers).
      loadVoiceSettings()
        .then(vs => {
          setVoiceSettings(vs);
          // Seed the routing dropdowns from the registry on first load.
          // Use the effective provider string from the core config.
          const slugs = new Set(vs.voiceProviders.map(p => p.slug));
          const sttStr =
            vs.sttProvider.kind === 'cloud'
              ? 'cloud'
              : vs.sttProvider.kind === 'local'
                ? vs.sttProvider.engine
                : slugs.has(vs.sttProvider.providerSlug)
                  ? vs.sttProvider.providerSlug
                  : 'cloud';
          const ttsStr =
            vs.ttsProvider.kind === 'cloud'
              ? 'cloud'
              : vs.ttsProvider.kind === 'local'
                ? vs.ttsProvider.engine
                : slugs.has(vs.ttsProvider.providerSlug)
                  ? vs.ttsProvider.providerSlug
                  : 'cloud';
          setSttProvider(prev => prev || sttStr);
          setTtsProvider(prev => prev || ttsStr);
          setSavedSttProvider(sttStr);
          setSavedTtsProvider(ttsStr);
        })
        .catch(err => {
          log('[VoicePanel] voice settings load failed (expected on older cores) %o', err);
          // Fallback: seed from legacy voice_status
          if (voiceResponse.stt_provider) {
            const seeded = voiceResponse.stt_provider === 'whisper' ? 'whisper' : 'cloud';
            setSttProvider(prev => prev || seeded);
          }
          if (voiceResponse.tts_provider) {
            const seeded = voiceResponse.tts_provider === 'piper' ? 'piper' : 'cloud';
            setTtsProvider(prev => prev || seeded);
          }
        });
      setError(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : t('voice.failedToLoadSettings');
      setError(message);
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    void loadData(true);
  }, []);

  const shouldPollWhisperInstall = whisperInstall?.state === 'installing';
  const shouldPollPiperInstall = piperInstall?.state === 'installing';

  useEffect(() => {
    if (!shouldPollWhisperInstall && !shouldPollPiperInstall) return;

    let cancelled = false;
    let inFlight = false;
    const pollInstallStatuses = async () => {
      if (inFlight) return;
      inFlight = true;
      try {
        const [nextWhisperStatus, nextPiperStatus] = await Promise.all([
          shouldPollWhisperInstall
            ? whisperInstallStatus().catch(err => {
                log('[voice-install:whisper] status poll failed %o', err);
                return null;
              })
            : Promise.resolve(null),
          shouldPollPiperInstall
            ? piperInstallStatus().catch(err => {
                log('[voice-install:piper] status poll failed %o', err);
                return null;
              })
            : Promise.resolve(null),
        ]);

        if (cancelled) return;
        if (nextWhisperStatus) setWhisperInstall(nextWhisperStatus);
        if (nextPiperStatus) setPiperInstall(nextPiperStatus);
      } finally {
        inFlight = false;
      }
    };

    void pollInstallStatuses();
    const intervalId = window.setInterval(() => {
      void pollInstallStatuses();
    }, LOCAL_INSTALL_STATUS_POLL_MS);

    return () => {
      cancelled = true;
      window.clearInterval(intervalId);
    };
  }, [shouldPollWhisperInstall, shouldPollPiperInstall]);

  const persistProviders = async (
    update: Partial<VoiceProvidersSnapshot> & {
      stt_provider?: string;
      tts_provider?: string;
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
      log('[VoicePanel:providers] saved %o', snapshot);
      setNotice(t('voice.providers.saved'));
      // Force a reload so the rest of the panel reflects the new state.
      await loadData(true);
    } catch (err) {
      const message = err instanceof Error ? err.message : t('voice.providers.failedToSave');
      setError(message);
    } finally {
      setIsSavingProviders(false);
    }
  };

  const sttExternalProviders = (voiceSettings?.voiceProviders ?? []).filter(
    p => p.capability === 'stt' || p.capability === 'both'
  );
  const ttsExternalProviders = (voiceSettings?.voiceProviders ?? []).filter(
    p => p.capability === 'tts' || p.capability === 'both'
  );

  const onSttProviderChange = (next: string) => {
    setSttProvider(next);
  };
  const onTtsProviderChange = (next: string) => {
    setTtsProvider(next);
  };

  const hasRoutingChanges = sttProvider !== savedSttProvider || ttsProvider !== savedTtsProvider;

  const saveRouting = useCallback(async () => {
    setIsSavingRouting(true);
    setError(null);
    try {
      await persistProviders({ stt_provider: sttProvider, tts_provider: ttsProvider });
      setSavedSttProvider(sttProvider);
      setSavedTtsProvider(ttsProvider);
      setNotice(t('voice.providers.saved'));
      void loadData(true);
    } catch (err) {
      setError(err instanceof Error ? err.message : t('voice.providers.failedToSave'));
    } finally {
      setIsSavingRouting(false);
    }
  }, [sttProvider, ttsProvider, persistProviders, t]);

  /**
   * Enable an external voice provider chip using the inline key form.
   * Called after the user enters an API key and clicks Save.
   */
  const handleEnableExternalProvider = useCallback(
    async (slug: string, apiKey: string) => {
      if (!voiceSettings) return;
      setIsSavingPendingKey(true);
      setError(null);
      try {
        const meta = BUILTIN_VOICE_PROVIDER_META[slug];
        const BUILTIN_ENDPOINTS: Record<string, string> = {
          deepgram: 'https://api.deepgram.com/v1',
          elevenlabs: 'https://api.elevenlabs.io/v1',
          openai: 'https://api.openai.com/v1',
        };
        const newProvider: VoiceProviderView = {
          id: '',
          slug,
          label: meta?.label ?? slug,
          endpoint: BUILTIN_ENDPOINTS[slug] ?? '',
          auth_style: 'bearer',
          capability: meta?.capability ?? 'both',
          stt_api_style: slug === 'deepgram' ? 'deepgram' : 'openai_audio',
          tts_api_style: slug === 'elevenlabs' ? 'elevenlabs' : 'openai_audio',
          default_stt_model:
            slug === 'deepgram'
              ? 'nova-2'
              : slug === 'openai'
                ? 'whisper-1'
                : slug === 'elevenlabs'
                  ? 'scribe_v1'
                  : null,
          default_tts_voice:
            slug === 'openai' ? 'alloy' : slug === 'elevenlabs' ? 'JBFqnCBsd6RMkjVDRZzb' : null,
          has_api_key: false,
        };
        if (apiKey) {
          await setVoiceProviderKey(slug, apiKey);
          newProvider.has_api_key = true;
        }
        const updated: VoiceSettings = {
          ...voiceSettings,
          voiceProviders: [
            ...voiceSettings.voiceProviders.filter(p => p.slug !== slug),
            newProvider,
          ],
        };
        await saveVoiceSettings(voiceSettings, updated);
        setVoiceSettings(updated);
        setPendingKeySlug(null);
        setPendingKeyValue('');
        setNotice(t('voice.providers.saved'));
        log('[VoicePanel:chip] enabled external provider %s', slug);
      } catch (err) {
        setError(err instanceof Error ? err.message : t('voice.providers.failedToSave'));
      } finally {
        setIsSavingPendingKey(false);
      }
    },
    [voiceSettings, t]
  );

  const handleRemoveProvider = useCallback(
    async (slug: string) => {
      if (!voiceSettings) return;
      try {
        await clearVoiceProviderKey(slug);
        const updated: VoiceSettings = {
          ...voiceSettings,
          voiceProviders: voiceSettings.voiceProviders.filter(p => p.slug !== slug),
        };
        await saveVoiceSettings(voiceSettings, updated);
        setVoiceSettings(updated);
        setNotice(t('voice.providers.saved'));
      } catch (err) {
        setError(err instanceof Error ? err.message : t('voice.providers.failedToSave'));
      }
    },
    [voiceSettings, t]
  );

  // Mascot voice picker moved to MascotPanel — see
  // `app/src/components/settings/panels/MascotPanel.tsx`. The voice id,
  // gender, and locale-default toggle all live in `mascotSlice`; this
  // panel only handles Piper / Whisper / dictation now.

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
      const pct =
        typeof status.progress === 'number' ? `${status.progress}%` : t('voice.providers.ellipsis');
      return `${t('voice.providers.installing')} ${pct}`;
    }
    if (busy) return t('voice.providers.installingBusy');
    if (status?.state === 'installed') return t('voice.providers.reinstallLocally');
    if (status?.state === 'broken') return t('voice.providers.repair');
    if (status?.state === 'error') return t('voice.providers.retryLocally');
    return t('voice.providers.installLocally');
  };

  const installStatusText = (status: VoiceInstallStatus | null, ready: boolean): string => {
    if (status?.state === 'installing') {
      const progress =
        typeof status.progress === 'number'
          ? `${t('voice.providers.installing')} ${status.progress}%`
          : t('voice.providers.installing');
      return status.stage ? `${progress} · ${status.stage}` : progress;
    }
    if (ready) return t('voice.providers.installed');
    if (status?.state === 'error' || status?.state === 'broken') {
      return status.error_detail ?? t('voice.providers.installFailed');
    }
    return t('voice.providers.notInstalled');
  };

  const installStatusClassName = (status: VoiceInstallStatus | null, ready: boolean): string => {
    if (status?.state === 'error' || status?.state === 'broken') {
      return 'text-red-600 dark:text-red-300';
    }
    if (status?.state === 'installing') return 'text-amber-600 dark:text-amber-300';
    if (ready) return 'text-emerald-600 dark:text-emerald-300';
    return 'text-neutral-500 dark:text-neutral-400';
  };

  const handleInstallWhisper = async () => {
    setIsInstallingWhisper(true);
    setError(null);
    setNotice(null);
    try {
      const force = whisperInstall?.state === 'installed';
      log('[voice-install:whisper] install click force=%s', force);
      const result = await installWhisper({ modelSize: sttModel || undefined, force });
      setWhisperInstall(result);
      setNotice(
        result.state === 'installed'
          ? t('voice.providers.whisperReady')
          : `${t('voice.providers.whisperInstallStarted')} (${result.stage ?? t('voice.providers.queued')})`
      );
    } catch (err) {
      const message =
        err instanceof Error ? err.message : t('voice.providers.failedToInstallWhisper');
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
      log('[voice-install:piper] install click force=%s', force);
      const result = await installPiper({ voiceId: ttsVoice || undefined, force });
      setPiperInstall(result);
      setNotice(
        result.state === 'installed'
          ? t('voice.providers.piperReady')
          : `${t('voice.providers.piperInstallStarted')} (${result.stage ?? t('voice.providers.queued')})`
      );
    } catch (err) {
      const message =
        err instanceof Error ? err.message : t('voice.providers.failedToInstallPiper');
      setError(message);
    } finally {
      setIsInstallingPiper(false);
      await loadData(false);
    }
  };

  const whisperReady =
    whisperInstall?.state !== 'installing' &&
    (whisperInstall?.state === 'installed' || Boolean(voiceStatus?.stt_available));
  const piperReady =
    piperInstall?.state !== 'installing' &&
    (piperInstall?.state === 'installed' || Boolean(voiceStatus?.tts_available));
  const pendingLocalProviderReady =
    pendingKeySlug === 'whisper' ? whisperReady : pendingKeySlug === 'piper' ? piperReady : true;

  return (
    <PanelPage
      className="z-10"
      contentClassName=""
      description={embedded ? undefined : t('pages.settings.ai.voiceDesc')}
      leading={embedded ? undefined : <SettingsBackButton onBack={navigateBack} />}>
      <div className={embedded ? 'space-y-4' : 'p-4 space-y-4'}>
        {/* Always-on listening moved to Settings → Features → Desktop Agent. */}

        {/* ─── Section 1: Voice Provider Chips ─────────────────────────── */}
        {/* Provider chips are intentional bespoke UI — kept as-is. */}
        <SettingsSection title={t('voice.providers.title')} description={t('voice.providers.desc')}>
          <div className="px-4 py-3" data-testid="voice-providers-section">
            {/* Chip row */}
            <div className="flex flex-wrap gap-2">
              {/* Cloud — always enabled, locked */}
              <div className="inline-flex items-center gap-2 rounded-full px-2.5 py-1 text-xs font-medium ring-1 ring-emerald-200 bg-emerald-50 text-emerald-800 dark:bg-emerald-500/10 dark:text-emerald-100 dark:ring-emerald-700">
                <span>{t('voice.providers.chip.cloud')}</span>
                <button
                  type="button"
                  role="switch"
                  aria-checked={true}
                  aria-label={t('voice.providers.chip.cloudAria')}
                  disabled
                  className="relative inline-flex h-4 w-7 shrink-0 items-center rounded-full bg-emerald-500 disabled:cursor-not-allowed">
                  <span
                    aria-hidden
                    className="inline-block h-3 w-3 transform rounded-full bg-white shadow translate-x-3.5"
                  />
                </button>
              </div>

              {/* Whisper — local STT, no API key required. Chip opens the
                  install/enable modal (which calls voice_install_whisper and
                  then voice_update_provider_settings on Enable). Toggling
                  off routes STT back to the managed cloud provider. */}
              {(() => {
                const tone = LOCAL_VOICE_PROVIDER_TONE.whisper;
                const enabled = sttProvider === 'whisper';
                return (
                  <div
                    className={`inline-flex items-center gap-2 rounded-full px-2.5 py-1 text-xs font-medium ring-1 transition-colors ${tone}`}>
                    <span>{t('voice.providers.chip.whisper')}</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={enabled}
                      data-testid="voice-provider-chip-whisper"
                      aria-label={
                        enabled
                          ? `${t('voice.providers.chip.disableProvider')} ${t('voice.providers.chip.whisper')}`
                          : `${t('voice.providers.chip.enableProvider')} ${t('voice.providers.chip.whisper')}`
                      }
                      // Stay disabled for the full install window: the
                      // local RPC kickoff (`isInstallingWhisper`) ends as
                      // soon as the start call returns, but the install
                      // itself continues until `voice_install_status`
                      // reports `installed` / `error`. Combining both
                      // signals prevents routing edits mid-install.
                      disabled={isInstallingWhisper || whisperInstall?.state === 'installing'}
                      onClick={() => {
                        if (enabled) {
                          onSttProviderChange('cloud');
                        } else {
                          setPendingKeySlug('whisper');
                          setPendingKeyValue('');
                        }
                      }}
                      className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors disabled:cursor-not-allowed disabled:opacity-60 ${enabled ? 'bg-primary-500' : 'bg-neutral-300 dark:bg-neutral-600'}`}>
                      <span
                        aria-hidden
                        className={`inline-block h-3 w-3 transform rounded-full bg-white shadow transition-transform ${enabled ? 'translate-x-3.5' : 'translate-x-0.5'}`}
                      />
                    </button>
                  </div>
                );
              })()}

              {/* Piper — local TTS, no API key required. Same chip flow as
                  Whisper above; targets the TTS routing slot. */}
              {(() => {
                const tone = LOCAL_VOICE_PROVIDER_TONE.piper;
                const enabled = ttsProvider === 'piper';
                return (
                  <div
                    className={`inline-flex items-center gap-2 rounded-full px-2.5 py-1 text-xs font-medium ring-1 transition-colors ${tone}`}>
                    <span>{t('voice.providers.chip.piper')}</span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={enabled}
                      data-testid="voice-provider-chip-piper"
                      aria-label={
                        enabled
                          ? `${t('voice.providers.chip.disableProvider')} ${t('voice.providers.chip.piper')}`
                          : `${t('voice.providers.chip.enableProvider')} ${t('voice.providers.chip.piper')}`
                      }
                      // Same install-window guard as the Whisper chip.
                      disabled={isInstallingPiper || piperInstall?.state === 'installing'}
                      onClick={() => {
                        if (enabled) {
                          onTtsProviderChange('cloud');
                        } else {
                          setPendingKeySlug('piper');
                          setPendingKeyValue('');
                        }
                      }}
                      className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors disabled:cursor-not-allowed disabled:opacity-60 ${enabled ? 'bg-primary-500' : 'bg-neutral-300 dark:bg-neutral-600'}`}>
                      <span
                        aria-hidden
                        className={`inline-block h-3 w-3 transform rounded-full bg-white shadow transition-transform ${enabled ? 'translate-x-3.5' : 'translate-x-0.5'}`}
                      />
                    </button>
                  </div>
                );
              })()}

              {/* External providers: Deepgram, ElevenLabs, OpenAI */}
              {(
                Object.entries(BUILTIN_VOICE_PROVIDER_META) as Array<
                  [
                    string,
                    {
                      label: string;
                      tone: string;
                      capability: 'stt' | 'tts' | 'both';
                      comingSoon?: boolean;
                    },
                  ]
                >
              ).map(([slug, meta]) => {
                const enabled = (voiceSettings?.voiceProviders ?? []).some(p => p.slug === slug);
                return (
                  <div
                    key={slug}
                    className={`inline-flex items-center gap-2 rounded-full px-2.5 py-1 text-xs font-medium ring-1 transition-colors ${meta.comingSoon ? 'opacity-60' : ''} ${meta.tone}`}>
                    <span>
                      {meta.label}
                      {meta.comingSoon && (
                        <span className="ml-1 text-[10px] opacity-70">
                          ({t('voice.providers.chip.comingSoon')})
                        </span>
                      )}
                    </span>
                    <button
                      type="button"
                      role="switch"
                      aria-checked={enabled}
                      data-testid={`voice-provider-chip-${slug}`}
                      aria-label={
                        enabled
                          ? `${t('voice.providers.chip.disableProvider')} ${meta.label}`
                          : `${t('voice.providers.chip.enableProvider')} ${meta.label}`
                      }
                      disabled={isSavingPendingKey || !!meta.comingSoon}
                      onClick={() => {
                        if (meta.comingSoon) return;
                        if (enabled) {
                          void handleRemoveProvider(slug);
                          if (sttProvider === slug) onSttProviderChange('cloud');
                          if (ttsProvider === slug) onTtsProviderChange('cloud');
                        } else {
                          setPendingKeySlug(slug);
                          setPendingKeyValue('');
                        }
                      }}
                      className={`relative inline-flex h-4 w-7 shrink-0 items-center rounded-full transition-colors disabled:cursor-not-allowed disabled:opacity-60 ${enabled ? 'bg-primary-500' : 'bg-neutral-300 dark:bg-neutral-600'}`}>
                      <span
                        aria-hidden
                        className={`inline-block h-3 w-3 transform rounded-full bg-white shadow transition-transform ${enabled ? 'translate-x-3.5' : 'translate-x-0.5'}`}
                      />
                    </button>
                  </div>
                );
              })}
            </div>
          </div>
        </SettingsSection>

        {/* ─── API Key Modal ──────────────────────────────────────────── */}
        {pendingKeySlug && (
          <div
            className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 dark:bg-black/60"
            onClick={e => {
              if (e.target === e.currentTarget && !isSavingPendingKey) {
                setPendingKeySlug(null);
                setPendingKeyValue('');
                setKeyTestResult(null);
              }
            }}
            data-testid="voice-provider-key-modal">
            <div className="w-full max-w-md rounded-2xl border border-neutral-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 shadow-xl p-6 space-y-4">
              {pendingKeySlug === 'whisper' || pendingKeySlug === 'piper' ? (
                /* ── Local provider modal (Whisper / Piper) ──────────── */
                <>
                  <div>
                    <h3 className="text-base font-semibold text-neutral-800 dark:text-neutral-100">
                      {t('voice.modal.title')}{' '}
                      {pendingKeySlug === 'whisper'
                        ? t('voice.providers.chip.whisper')
                        : t('voice.providers.chip.piper')}
                    </h3>
                    <p className="text-xs text-neutral-500 dark:text-neutral-400 mt-1">
                      {pendingKeySlug === 'whisper'
                        ? t('voice.modal.whisperDesc')
                        : t('voice.modal.piperDesc')}
                    </p>
                  </div>

                  {pendingKeySlug === 'whisper' && (
                    <label className="block space-y-1">
                      <span className="text-xs font-medium text-neutral-500 dark:text-neutral-300">
                        {t('voice.providers.whisperModel')}
                      </span>
                      <SettingsSelect
                        value={sttModel || 'medium'}
                        onChange={e => setSttModel(e.target.value)}
                        className="w-full">
                        <option value="tiny">{t('voice.providers.whisperModelTiny')}</option>
                        <option value="base">{t('voice.providers.whisperModelBase')}</option>
                        <option value="small">{t('voice.providers.whisperModelSmall')}</option>
                        <option value="medium">{t('voice.providers.whisperModelMedium')}</option>
                        <option value="whisper-large-v3-turbo">
                          {t('voice.providers.whisperModelLargeTurbo')}
                        </option>
                      </SettingsSelect>
                    </label>
                  )}

                  {pendingKeySlug === 'piper' && (
                    <label className="block space-y-1">
                      <span className="text-xs font-medium text-neutral-500 dark:text-neutral-300">
                        {t('voice.providers.piperVoice')}
                      </span>
                      <SettingsSelect
                        value={
                          PIPER_VOICE_PRESET_IDS.some(v => v === ttsVoice) ? ttsVoice : '__custom__'
                        }
                        onChange={e => {
                          if (e.target.value !== '__custom__') setTtsVoice(e.target.value);
                        }}
                        className="w-full">
                        {piperVoicePresets.map(v => (
                          <option key={v.id} value={v.id}>
                            {v.label}
                          </option>
                        ))}
                        <option value="__custom__">{t('voice.providers.customVoiceOption')}</option>
                      </SettingsSelect>
                    </label>
                  )}

                  {/* Install status */}
                  {pendingKeySlug === 'whisper' && (
                    <div className="flex items-center gap-2">
                      <Button
                        type="button"
                        variant={whisperReady ? 'secondary' : 'primary'}
                        size="xs"
                        onClick={() => void handleInstallWhisper()}
                        disabled={isInstallingWhisper || whisperInstall?.state === 'installing'}>
                        {installButtonLabel(whisperInstall, isInstallingWhisper, 'Whisper')}
                      </Button>
                      <span
                        className={`text-[11px] ${installStatusClassName(whisperInstall, whisperReady)}`}>
                        {installStatusText(whisperInstall, whisperReady)}
                      </span>
                    </div>
                  )}

                  {pendingKeySlug === 'piper' && (
                    <div className="flex items-center gap-2">
                      <Button
                        type="button"
                        variant={piperReady ? 'secondary' : 'primary'}
                        size="xs"
                        onClick={() => void handleInstallPiper()}
                        disabled={isInstallingPiper || piperInstall?.state === 'installing'}>
                        {installButtonLabel(piperInstall, isInstallingPiper, 'Piper')}
                      </Button>
                      <span
                        className={`text-[11px] ${installStatusClassName(piperInstall, piperReady)}`}>
                        {installStatusText(piperInstall, piperReady)}
                      </span>
                    </div>
                  )}

                  <div className="flex items-center justify-between pt-2">
                    <Button
                      type="button"
                      variant="secondary"
                      size="xs"
                      onClick={() => {
                        setPendingKeySlug(null);
                        setKeyTestResult(null);
                      }}>
                      {t('common.cancel')}
                    </Button>
                    <Button
                      type="button"
                      variant="primary"
                      size="xs"
                      onClick={() => {
                        if (!pendingLocalProviderReady) return;
                        if (pendingKeySlug === 'whisper') {
                          onSttProviderChange('whisper');
                          if (sttModel) void persistProviders({ stt_model: sttModel });
                        } else {
                          onTtsProviderChange('piper');
                          if (ttsVoice) void persistProviders({ tts_voice: ttsVoice });
                        }
                        setPendingKeySlug(null);
                        setKeyTestResult(null);
                      }}
                      disabled={!pendingLocalProviderReady || isSavingProviders}>
                      {t('voice.modal.enable')}
                    </Button>
                  </div>
                </>
              ) : (
                /* ── External provider modal (API key) ───────────────── */
                <>
                  <div>
                    <h3 className="text-base font-semibold text-neutral-800 dark:text-neutral-100">
                      {t('voice.modal.title')}{' '}
                      {BUILTIN_VOICE_PROVIDER_META[pendingKeySlug]?.label ?? pendingKeySlug}
                    </h3>
                    <p className="text-xs text-neutral-500 dark:text-neutral-400 mt-1">
                      {t('voice.modal.desc')}
                    </p>
                  </div>

                  <label className="block space-y-1">
                    <span className="text-xs font-medium text-neutral-500 dark:text-neutral-300">
                      {t('voice.providers.chip.apiKeyLabel')}
                    </span>
                    <SettingsTextField
                      id="voice-provider-key-input"
                      type="password"
                      autoComplete="off"
                      autoCorrect="off"
                      spellCheck={false}
                      data-form-type="other"
                      data-lpignore="true"
                      value={pendingKeyValue}
                      onChange={e => {
                        setPendingKeyValue(e.target.value);
                        setKeyTestResult(null);
                      }}
                      disabled={isSavingPendingKey}
                      placeholder={t('voice.providers.chip.apiKeyPlaceholder')}
                      className="w-full"
                    />
                  </label>

                  {keyTestResult && (
                    <div
                      className={`rounded-md px-3 py-2 text-xs ${
                        keyTestResult.ok
                          ? 'bg-emerald-50 text-emerald-700 border border-emerald-200 dark:bg-emerald-500/10 dark:text-emerald-300 dark:border-emerald-500/30'
                          : 'bg-red-50 text-red-700 border border-red-200 dark:bg-red-500/10 dark:text-red-300 dark:border-red-500/30'
                      }`}>
                      {keyTestResult.detail}
                    </div>
                  )}

                  <div className="flex items-center justify-between pt-2">
                    <Button
                      type="button"
                      variant="secondary"
                      size="xs"
                      onClick={() => {
                        setPendingKeySlug(null);
                        setPendingKeyValue('');
                        setKeyTestResult(null);
                      }}
                      disabled={isSavingPendingKey}>
                      {t('common.cancel')}
                    </Button>

                    <div className="flex items-center gap-2">
                      <Button
                        type="button"
                        variant="secondary"
                        size="xs"
                        disabled={!pendingKeyValue.trim() || isTestingKey || isSavingPendingKey}
                        onClick={async () => {
                          if (!pendingKeySlug || !pendingKeyValue.trim()) return;
                          setIsTestingKey(true);
                          setKeyTestResult(null);
                          try {
                            await handleEnableExternalProvider(pendingKeySlug, pendingKeyValue);
                            setPendingKeySlug(pendingKeySlug);
                            const meta = BUILTIN_VOICE_PROVIDER_META[pendingKeySlug];
                            const workload = meta?.capability === 'tts' ? 'tts' : 'stt';
                            const result = await testVoiceProvider(
                              workload as 'stt' | 'tts',
                              pendingKeySlug,
                              true
                            );
                            setKeyTestResult(result);
                          } catch (err) {
                            setPendingKeySlug(pendingKeySlug);
                            setKeyTestResult({
                              ok: false,
                              detail: err instanceof Error ? err.message : 'Test failed',
                            });
                          } finally {
                            setIsTestingKey(false);
                          }
                        }}>
                        {isTestingKey ? t('voice.modal.testing') : t('voice.modal.testKey')}
                      </Button>
                      <Button
                        type="button"
                        variant="primary"
                        size="xs"
                        onClick={() =>
                          void handleEnableExternalProvider(pendingKeySlug, pendingKeyValue)
                        }
                        disabled={!pendingKeyValue.trim() || isSavingPendingKey}>
                        {isSavingPendingKey ? t('common.loading') : t('voice.modal.saveAndEnable')}
                      </Button>
                    </div>
                  </div>
                </>
              )}
            </div>
          </div>
        )}

        {/* ─── Section 2: Voice Routing ─────────────────────────────────── */}
        <SettingsSection title={t('voice.routing.title')} description={t('voice.routing.desc')}>
          <SettingsRow
            stacked
            control={
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                {/* STT routing */}
                <div className="space-y-2">
                  <label className="block space-y-1">
                    <span className="text-xs font-medium text-neutral-500 dark:text-neutral-300">
                      {t('voice.providers.sttProvider')}
                    </span>
                    <SettingsSelect
                      aria-label={t('voice.providers.sttProviderAria')}
                      data-testid="stt-provider-select"
                      value={sttProvider || 'cloud'}
                      disabled={isSavingProviders}
                      onChange={e => onSttProviderChange(e.target.value)}
                      className="w-full">
                      <option value="cloud">{t('voice.providers.cloudWhisperProxy')}</option>
                      {/* Whisper only shown when enabled */}
                      {(sttProvider === 'whisper' ||
                        (voiceSettings?.voiceProviders ?? []).some(p => p.slug === 'whisper')) && (
                        <option value="whisper">{t('voice.providers.localWhisper')}</option>
                      )}
                      {/* External providers that support STT */}
                      {sttExternalProviders.map(p => (
                        <option key={p.slug} value={p.slug}>
                          {p.label}
                        </option>
                      ))}
                    </SettingsSelect>
                  </label>

                  <div className="flex items-center gap-2">
                    <Button
                      type="button"
                      variant="secondary"
                      size="xs"
                      data-testid="test-stt-button"
                      disabled={isTestingStt || !sttProvider}
                      onClick={async () => {
                        setIsTestingStt(true);
                        setSttTestResult(null);
                        try {
                          const result = await testVoiceProvider('stt', sttProvider || 'cloud');
                          setSttTestResult(result);
                        } catch (err) {
                          setSttTestResult({
                            ok: false,
                            detail: err instanceof Error ? err.message : 'Test failed',
                          });
                        } finally {
                          setIsTestingStt(false);
                        }
                      }}>
                      {isTestingStt ? t('voice.modal.testing') : t('voice.routing.testStt')}
                    </Button>
                    {sttTestResult && (
                      <span
                        className={`text-[11px] ${
                          sttTestResult.ok
                            ? 'text-emerald-600 dark:text-emerald-300'
                            : 'text-red-600 dark:text-red-300'
                        }`}>
                        {sttTestResult.detail}
                      </span>
                    )}
                  </div>

                  {/* Whisper model picker — shown when Whisper is selected */}
                  {sttProvider === 'whisper' && (
                    <label className="block space-y-1">
                      <span className="text-xs font-medium text-neutral-500 dark:text-neutral-300">
                        {t('voice.providers.whisperModel')}
                      </span>
                      <SettingsSelect
                        aria-label={t('voice.providers.whisperModelAria')}
                        data-testid="stt-model-select"
                        value={sttModel || 'medium'}
                        disabled={isSavingProviders}
                        onChange={e => {
                          const nextModel = e.target.value;
                          setSttModel(nextModel);
                          void persistProviders({ stt_model: nextModel });
                        }}
                        className="w-full">
                        <option value="tiny">{t('voice.providers.whisperModelTiny')}</option>
                        <option value="base">{t('voice.providers.whisperModelBase')}</option>
                        <option value="small">{t('voice.providers.whisperModelSmall')}</option>
                        <option value="medium">{t('voice.providers.whisperModelMedium')}</option>
                        <option value="whisper-large-v3-turbo">
                          {t('voice.providers.whisperModelLargeTurbo')}
                        </option>
                      </SettingsSelect>
                    </label>
                  )}
                </div>

                {/* TTS routing */}
                <div className="space-y-2">
                  <label className="block space-y-1">
                    <span className="text-xs font-medium text-neutral-500 dark:text-neutral-300">
                      {t('voice.providers.ttsProvider')}
                    </span>
                    <SettingsSelect
                      aria-label={t('voice.providers.ttsProviderAria')}
                      data-testid="tts-provider-select"
                      value={ttsProvider || 'cloud'}
                      disabled={isSavingProviders}
                      onChange={e => onTtsProviderChange(e.target.value)}
                      className="w-full">
                      <option value="cloud">{t('voice.providers.cloudElevenLabsProxy')}</option>
                      {/* Piper only shown when enabled */}
                      {(ttsProvider === 'piper' ||
                        (voiceSettings?.voiceProviders ?? []).some(p => p.slug === 'piper')) && (
                        <option value="piper">{t('voice.providers.localPiper')}</option>
                      )}
                      {/* External providers that support TTS */}
                      {ttsExternalProviders.map(p => (
                        <option key={p.slug} value={p.slug}>
                          {p.label}
                        </option>
                      ))}
                    </SettingsSelect>
                  </label>

                  <div className="flex items-center gap-2">
                    <Button
                      type="button"
                      variant="secondary"
                      size="xs"
                      data-testid="test-tts-button"
                      disabled={isTestingTts || !ttsProvider}
                      onClick={async () => {
                        setIsTestingTts(true);
                        setTtsTestResult(null);
                        try {
                          // For ElevenLabs, include the voice ID so the test
                          // actually synthesizes audio with the selected voice.
                          let ttsTestProvider = ttsProvider || 'cloud';
                          if (ttsProvider === 'elevenlabs' && elevenlabsVoiceId) {
                            ttsTestProvider = `elevenlabs:${elevenlabsVoiceId}`;
                          }
                          const result = await testVoiceProvider('tts', ttsTestProvider);
                          setTtsTestResult(result);
                        } catch (err) {
                          setTtsTestResult({
                            ok: false,
                            detail: err instanceof Error ? err.message : 'Test failed',
                          });
                        } finally {
                          setIsTestingTts(false);
                        }
                      }}>
                      {isTestingTts ? t('voice.modal.testing') : t('voice.routing.testTts')}
                    </Button>
                    {ttsTestResult && (
                      <span
                        className={`text-[11px] ${
                          ttsTestResult.ok
                            ? 'text-emerald-600 dark:text-emerald-300'
                            : 'text-red-600 dark:text-red-300'
                        }`}>
                        {ttsTestResult.detail}
                      </span>
                    )}
                  </div>

                  {/* Piper voice picker — shown when Piper is selected */}
                  {ttsProvider === 'piper' && (
                    <label className="block space-y-1">
                      <span className="text-xs font-medium text-neutral-500 dark:text-neutral-300">
                        {t('voice.providers.piperVoice')}
                      </span>
                      <SettingsSelect
                        aria-label={t('voice.providers.piperVoiceAria')}
                        data-testid="tts-voice-select"
                        value={
                          PIPER_VOICE_PRESET_IDS.some(v => v === ttsVoice) ? ttsVoice : '__custom__'
                        }
                        disabled={isSavingProviders}
                        onChange={e => {
                          const next = e.target.value;
                          if (next === '__custom__') return;
                          setTtsVoice(next);
                          void persistProviders({ tts_voice: next });
                          void installPiper({ voiceId: next }).catch(err =>
                            console.warn(
                              '[voice-install:piper] auto-install on voice change failed:',
                              err
                            )
                          );
                        }}
                        className="w-full">
                        {piperVoicePresets.map(v => (
                          <option key={v.id} value={v.id}>
                            {v.label}
                          </option>
                        ))}
                        <option value="__custom__">{t('voice.providers.customVoiceOption')}</option>
                      </SettingsSelect>
                      {!PIPER_VOICE_PRESET_IDS.some(v => v === ttsVoice) && (
                        <SettingsTextField
                          aria-label={t('voice.providers.customVoiceAria')}
                          data-testid="tts-voice-input"
                          value={ttsVoice}
                          placeholder={t('voice.providers.customVoicePlaceholder')}
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
                          className="mt-1 w-full"
                        />
                      )}
                      <p className="text-[11px] text-neutral-500 dark:text-neutral-400 mt-0.5">
                        {t('voice.providers.piperVoicesDesc')}
                      </p>
                    </label>
                  )}

                  {/* ElevenLabs voice picker — shown when ElevenLabs is selected for TTS */}
                  {ttsProvider === 'elevenlabs' && (
                    <label className="block space-y-1">
                      <span className="text-xs font-medium text-neutral-500 dark:text-neutral-300">
                        {t('voice.routing.elevenlabsVoice')}
                      </span>
                      <SettingsSelect
                        aria-label={t('voice.routing.elevenlabsVoiceAria')}
                        data-testid="elevenlabs-voice-select"
                        value={
                          isCuratedVoicePreset(elevenlabsVoiceId) ? elevenlabsVoiceId : '__custom__'
                        }
                        disabled={isSavingProviders}
                        onChange={e => {
                          const next = e.target.value;
                          if (next === '__custom__') return;
                          setElevenlabsVoiceId(next);
                        }}
                        className="w-full">
                        {ELEVENLABS_VOICE_PRESETS.map(v => (
                          <option key={v.id} value={v.id}>
                            {v.label}
                          </option>
                        ))}
                        <option value="__custom__">{t('voice.providers.customVoiceOption')}</option>
                      </SettingsSelect>
                      {!isCuratedVoicePreset(elevenlabsVoiceId) && (
                        <SettingsTextField
                          aria-label={t('voice.routing.elevenlabsVoiceIdAria')}
                          data-testid="elevenlabs-voice-input"
                          value={elevenlabsVoiceId}
                          placeholder="JBFqnCBsd6RMkjVDRZzb"
                          disabled={isSavingProviders}
                          onChange={e => setElevenlabsVoiceId(e.target.value)}
                          className="mt-1 w-full"
                        />
                      )}
                      <p className="text-[11px] text-neutral-500 dark:text-neutral-400 mt-0.5">
                        {t('voice.routing.elevenlabsVoiceDesc')}
                      </p>
                    </label>
                  )}
                </div>
              </div>
            }
          />
          <div className="flex justify-end px-4 py-3 border-t border-neutral-100 dark:border-neutral-800">
            <Button
              type="button"
              variant="primary"
              size="xs"
              data-testid="save-voice-routing"
              disabled={!hasRoutingChanges || isSavingRouting}
              onClick={() => void saveRouting()}>
              {isSavingRouting ? t('common.loading') : t('voice.routing.save')}
            </Button>
          </div>
        </SettingsSection>

        {/* ─── Section 3: Push-to-talk ─────────────────────────────────
            Global PTT hotkey + session preferences. The panel is
            self-contained — it only mutates the `ptt` slice, and
            `usePttHotkey` (T11) reacts to slice changes to (re)register
            the binding with the Tauri shell. Mounted here so users hunt
            for it under Voice settings alongside dictation. */}
        <PttSettingsPanel />

        {/* Mascot voice picker now lives in Mascot settings. Link
            kept here so users hunting in Voice settings can find it. */}
        {ttsProvider !== 'piper' && (
          <section data-testid="mascot-voice-link">
            <SettingsSection>
              <SettingsRow
                stacked
                label={t('voice.providers.mascotVoice')}
                control={
                  <p className="text-xs text-neutral-500 dark:text-neutral-400">
                    {t('voice.providers.mascotVoiceDescPrefix')}{' '}
                    <button
                      type="button"
                      onClick={() => navigateToSettings('personality#face')}
                      className="underline text-primary-600 dark:text-primary-300 hover:text-primary-700 dark:hover:text-primary-200">
                      {t('voice.providers.mascotSettings')}
                    </button>
                    {t('voice.providers.mascotVoiceDescSuffix')}
                  </p>
                }
              />
            </SettingsSection>
          </section>
        )}

        {/* Status line */}
        <SettingsStatusLine
          saving={isSavingProviders || isSavingRouting}
          savedNote={notice}
          error={error}
          savingLabel={t('common.loading')}
        />
      </div>
    </PanelPage>
  );
};

export default VoicePanel;
