import { useState } from 'react';

import { installPiper } from '../../../services/api/voiceInstallApi';
import { testVoiceProvider, type VoiceProviderView } from '../../../services/api/voiceSettingsApi';
import type { VoiceStatus } from '../../../utils/tauriCommands';
import { Button } from '../../ui';
import { SettingsRow, SettingsSection, SettingsSelect, SettingsTextField } from '../controls';
import { ELEVENLABS_VOICE_PRESETS, isCuratedVoicePreset } from './elevenlabsVoicePresets';

interface VoicePanelRoutingSectionProps {
  t: (key: string) => string;
  sttProvider: string;
  ttsProvider: string;
  onSttProviderChange: (next: string) => void;
  onTtsProviderChange: (next: string) => void;
  isSavingProviders: boolean;
  sttExternalProviders: VoiceProviderView[];
  ttsExternalProviders: VoiceProviderView[];
  piperEnabledElsewhere: boolean;
  ttsVoice: string;
  setTtsVoice: (value: string) => void;
  piperVoicePresets: ReadonlyArray<{ id: string; label: string }>;
  piperVoicePresetIds: readonly string[];
  voiceStatus: VoiceStatus | null;
  persistProviders: (update: { tts_voice?: string }) => Promise<void>;
  elevenlabsVoiceId: string;
  setElevenlabsVoiceId: (value: string) => void;
  ttsTestBlockedByInstall: boolean;
  hasRoutingChanges: boolean;
  isSavingRouting: boolean;
  saveRouting: () => Promise<void>;
}

/** STT/TTS provider routing pickers + per-workload test buttons. */
const VoicePanelRoutingSection = ({
  t,
  sttProvider,
  ttsProvider,
  onSttProviderChange,
  onTtsProviderChange,
  isSavingProviders,
  sttExternalProviders,
  ttsExternalProviders,
  piperEnabledElsewhere,
  ttsVoice,
  setTtsVoice,
  piperVoicePresets,
  piperVoicePresetIds,
  voiceStatus,
  persistProviders,
  elevenlabsVoiceId,
  setElevenlabsVoiceId,
  ttsTestBlockedByInstall,
  hasRoutingChanges,
  isSavingRouting,
  saveRouting,
}: VoicePanelRoutingSectionProps) => {
  const [isTestingStt, setIsTestingStt] = useState(false);
  const [sttTestResult, setSttTestResult] = useState<{ ok: boolean; detail: string } | null>(null);
  const [isTestingTts, setIsTestingTts] = useState(false);
  const [ttsTestResult, setTtsTestResult] = useState<{ ok: boolean; detail: string } | null>(null);

  return (
    <SettingsSection title={t('voice.routing.title')} description={t('voice.routing.desc')}>
      <SettingsRow
        stacked
        control={
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
            {/* STT routing */}
            <div className="space-y-2">
              <label className="block space-y-1">
                <span className="text-xs font-medium text-content-muted dark:text-content-secondary">
                  {t('voice.providers.sttProvider')}
                </span>
                <SettingsSelect
                  aria-label={t('voice.providers.sttProviderAria')}
                  data-testid="stt-provider-select"
                  value={sttProvider || 'cloud'}
                  disabled={isSavingProviders}
                  onChange={e => onSttProviderChange(e.target.value)}
                  className="w-full">
                  <option value="cloud">{t('voice.providers.backendSttProxy')}</option>
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
                        ? 'text-sage-600 dark:text-sage-300'
                        : 'text-coral-600 dark:text-coral-300'
                    }`}>
                    {sttTestResult.detail}
                  </span>
                )}
              </div>
            </div>

            {/* TTS routing */}
            <div className="space-y-2">
              <label className="block space-y-1">
                <span className="text-xs font-medium text-content-muted dark:text-content-secondary">
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
                  {(ttsProvider === 'piper' || piperEnabledElsewhere) && (
                    <option value="piper">{t('voice.providers.localPiper')}</option>
                  )}
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
                  disabled={isTestingTts || !ttsProvider || ttsTestBlockedByInstall}
                  title={ttsTestBlockedByInstall ? t('voice.providers.notInstalled') : undefined}
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
                        ? 'text-sage-600 dark:text-sage-300'
                        : 'text-coral-600 dark:text-coral-300'
                    }`}>
                    {ttsTestResult.detail}
                  </span>
                )}
              </div>

              {/* Piper voice picker — shown when Piper is selected */}
              {ttsProvider === 'piper' && (
                <label className="block space-y-1">
                  <span className="text-xs font-medium text-content-muted dark:text-content-secondary">
                    {t('voice.providers.piperVoice')}
                  </span>
                  <SettingsSelect
                    aria-label={t('voice.providers.piperVoiceAria')}
                    data-testid="tts-voice-select"
                    value={piperVoicePresetIds.some(v => v === ttsVoice) ? ttsVoice : '__custom__'}
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
                  {!piperVoicePresetIds.some(v => v === ttsVoice) && (
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
                  <p className="text-[11px] text-content-muted mt-0.5">
                    {t('voice.providers.piperVoicesDesc')}
                  </p>
                </label>
              )}

              {/* ElevenLabs voice picker — shown when ElevenLabs is selected for TTS */}
              {ttsProvider === 'elevenlabs' && (
                <label className="block space-y-1">
                  <span className="text-xs font-medium text-content-muted dark:text-content-secondary">
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
                  <p className="text-[11px] text-content-muted mt-0.5">
                    {t('voice.routing.elevenlabsVoiceDesc')}
                  </p>
                </label>
              )}
            </div>
          </div>
        }
      />
      <div className="flex justify-end px-4 py-3 border-t border-line-subtle">
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
  );
};

export default VoicePanelRoutingSection;
