import debug from 'debug';
import { ArrowRight } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  clearVoiceProviderKey,
  loadVoiceSettings,
  saveVoiceSettings,
  setVoiceProviderKey,
  type VoiceProviderView,
  type VoiceSettings,
} from '../../../services/api/voiceSettingsApi';
import {
  openhumanVoiceSetProviders,
  openhumanVoiceStatus,
  type VoiceProvidersSnapshot,
  type VoiceStatus,
} from '../../../utils/tauriCommands';
import PanelPage from '../../layout/PanelPage';
import { Button, Card, Field, StatusLine } from '../../ui';
import SettingsBackButton from '../components/SettingsBackButton';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import VoicePanelKeyModal from './VoicePanelKeyModal';
import VoicePanelProviderChips, { BUILTIN_VOICE_PROVIDER_META } from './VoicePanelProviderChips';
import VoicePanelRoutingSection from './VoicePanelRoutingSection';

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

const log = debug('voice:settings');

interface VoicePanelProps {
  /** When true, render without the SettingsHeader chrome (used when embedded
   *  inside the onboarding custom wizard). */
  embedded?: boolean;
  /** Let the host page own scrolling when this panel is embedded. */
  scrollable?: boolean;
}

const VoicePanel = ({ embedded = false, scrollable = true }: VoicePanelProps = {}) => {
  const { t } = useT();
  const { navigateBack, navigateToSettings } = useSettingsNavigation();
  const [voiceStatus, setVoiceStatus] = useState<VoiceStatus | null>(null);
  // Local provider selectors — initialised from voice_status, persisted via
  // openhumanVoiceSetProviders on change. Empty string until first load.
  const [sttProvider, setSttProvider] = useState<string>('');
  const [ttsProvider, setTtsProvider] = useState<string>('');
  const [savedSttProvider, setSavedSttProvider] = useState<string>('');
  const [savedTtsProvider, setSavedTtsProvider] = useState<string>('');
  const [isSavingRouting, setIsSavingRouting] = useState(false);
  const [ttsVoice, setTtsVoice] = useState<string>('');
  const [elevenlabsVoiceId, setElevenlabsVoiceId] = useState<string>('JBFqnCBsd6RMkjVDRZzb');
  const [isSavingProviders, setIsSavingProviders] = useState(false);
  const [isCheckingPiper, setIsCheckingPiper] = useState(false);
  const [, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  // Voice provider registry state
  const [voiceSettings, setVoiceSettings] = useState<VoiceSettings | null>(null);
  // Chip-toggle inline API-key form state
  const [pendingKeySlug, setPendingKeySlug] = useState<string | null>(null);
  const [pendingKeyValue, setPendingKeyValue] = useState('');
  const [isSavingPendingKey, setIsSavingPendingKey] = useState(false);
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

  const loadData = async () => {
    try {
      const voiceResponse = await openhumanVoiceStatus();
      setVoiceStatus(voiceResponse);
      // Seed the voice id from voice_status on first load only. There is no
      // STT counterpart: the model id belongs to whichever hosted engine is
      // selected and comes from its `voice_providers` entry.
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
              ? // `cloud` is a routing sentinel: it delegates to the configured
                // engine, which voice_status reports after resolving it. Seed the
                // selector with that effective engine so Settings does not claim
                // the backend proxy is in use when a hosted BYOK engine is.
                voiceResponse.stt_engine || 'cloud'
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
          // Fallback: seed from voice_status, which already reports the
          // resolved routing string for the selected STT engine.
          if (voiceResponse.stt_engine) {
            setSttProvider(prev => prev || voiceResponse.stt_engine);
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
    void loadData();
  }, []);

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
      await loadData();
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
      void loadData();
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
          stt_api_style:
            slug === 'deepgram'
              ? 'deepgram'
              : slug === 'elevenlabs'
                ? 'elevenlabs'
                : 'openai_audio',
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
  // panel only handles Piper / dictation now.

  // Piper is user-supplied: OpenHuman never downloads the binary or its
  // voices. Readiness is whatever `voice_status` resolves — a `piper` binary
  // (PATH or PIPER_BIN) plus the selected voice's .onnx file. After the user
  // installs Piper themselves, "Check again" re-reads that status.
  const handleRecheckPiper = async () => {
    setIsCheckingPiper(true);
    setError(null);
    setNotice(null);
    log('[voice:piper] re-checking user-supplied piper binary and voice');
    try {
      await loadData();
    } finally {
      setIsCheckingPiper(false);
    }
  };

  const piperReady = Boolean(voiceStatus?.tts_available);
  const pendingLocalProviderReady = pendingKeySlug === 'piper' ? piperReady : true;

  // Piper's Test button is useless until the user-supplied binary and voice
  // file resolve — exercising it just errors out on a missing binary or voice
  // file. STT has no local artifact at all (every engine is a hosted HTTP
  // call), so its Test button is never gated.
  const ttsTestBlockedByInstall = ttsProvider === 'piper' && !piperReady;

  return (
    <PanelPage
      className="z-10"
      contentClassName=""
      description={embedded ? undefined : t('pages.settings.ai.voiceDesc')}
      leading={embedded ? undefined : <SettingsBackButton onBack={navigateBack} />}
      scrollable={scrollable}>
      <div className={embedded ? 'space-y-5' : 'p-4 space-y-5'}>
        {/* ─── Section 1: Voice Provider Chips ─────────────────────────── */}
        <VoicePanelProviderChips
          t={t}
          sttProvider={sttProvider}
          ttsProvider={ttsProvider}
          onSttProviderChange={onSttProviderChange}
          onTtsProviderChange={onTtsProviderChange}
          voiceSettings={voiceSettings}
          isSavingPendingKey={isSavingPendingKey}
          setPendingKeySlug={setPendingKeySlug}
          setPendingKeyValue={setPendingKeyValue}
          handleRemoveProvider={handleRemoveProvider}
        />

        {/* ─── API Key Modal ──────────────────────────────────────────── */}
        {pendingKeySlug && (
          <VoicePanelKeyModal
            t={t}
            pendingKeySlug={pendingKeySlug}
            setPendingKeySlug={setPendingKeySlug}
            pendingKeyValue={pendingKeyValue}
            setPendingKeyValue={setPendingKeyValue}
            isSavingPendingKey={isSavingPendingKey}
            handleEnableExternalProvider={handleEnableExternalProvider}
            ttsVoice={ttsVoice}
            setTtsVoice={setTtsVoice}
            piperVoicePresets={piperVoicePresets}
            piperVoicePresetIds={PIPER_VOICE_PRESET_IDS}
            isCheckingPiper={isCheckingPiper}
            handleRecheckPiper={handleRecheckPiper}
            piperReady={piperReady}
            pendingLocalProviderReady={pendingLocalProviderReady}
            isSavingProviders={isSavingProviders}
            onTtsProviderChange={onTtsProviderChange}
            persistProviders={persistProviders}
          />
        )}

        {/* ─── Section 2: Voice Routing ─────────────────────────────────── */}
        <VoicePanelRoutingSection
          t={t}
          sttProvider={sttProvider}
          ttsProvider={ttsProvider}
          onSttProviderChange={onSttProviderChange}
          onTtsProviderChange={onTtsProviderChange}
          isSavingProviders={isSavingProviders}
          sttExternalProviders={sttExternalProviders}
          ttsExternalProviders={ttsExternalProviders}
          piperEnabledElsewhere={(voiceSettings?.voiceProviders ?? []).some(
            p => p.slug === 'piper'
          )}
          ttsVoice={ttsVoice}
          setTtsVoice={setTtsVoice}
          piperVoicePresets={piperVoicePresets}
          piperVoicePresetIds={PIPER_VOICE_PRESET_IDS}
          voiceStatus={voiceStatus}
          persistProviders={persistProviders}
          elevenlabsVoiceId={elevenlabsVoiceId}
          setElevenlabsVoiceId={setElevenlabsVoiceId}
          ttsTestBlockedByInstall={ttsTestBlockedByInstall}
          hasRoutingChanges={hasRoutingChanges}
          isSavingRouting={isSavingRouting}
          saveRouting={saveRouting}
        />

        {/* Mascot voice picker now lives in Mascot settings. Link
            kept here so users hunting in Voice settings can find it. */}
        {ttsProvider !== 'piper' && (
          <Card data-testid="mascot-voice-link">
            <Field
              label={t('voice.providers.mascotVoice')}
              description={t('voice.providers.mascotVoiceCardDesc')}
              control={
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  trailingIcon={<ArrowRight className="h-3.5 w-3.5" aria-hidden />}
                  onClick={() => navigateToSettings('face')}>
                  {t('voice.providers.mascotSettings')}
                </Button>
              }
            />
          </Card>
        )}

        {/* Status line */}
        <StatusLine
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
