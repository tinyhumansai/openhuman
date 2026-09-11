import type { VoiceInstallStatus } from '../../../services/api/voiceInstallApi';
import type { VoiceSettings } from '../../../services/api/voiceSettingsApi';
import { SettingsSection, SettingsSwitch } from '../controls';

/** Built-in voice provider slugs with display metadata. */
export const BUILTIN_VOICE_PROVIDER_META: Record<
  string,
  { label: string; capability: 'stt' | 'tts' | 'both'; comingSoon?: boolean }
> = {
  deepgram: { label: 'Deepgram', capability: 'stt', comingSoon: true },
  elevenlabs: { label: 'ElevenLabs', capability: 'both' },
  openai: { label: 'OpenAI', capability: 'both', comingSoon: true },
};

/** Shared pill shell for every provider chip — semantic tokens only. */
const CHIP_SHELL =
  'inline-flex items-center gap-2 rounded-full border px-2.5 py-1 text-xs font-medium transition-colors';
const CHIP_NEUTRAL = 'border-line bg-surface-subtle text-content-secondary dark:border-line-strong';
/** The cloud chip is always-on and locked — styled like `Badge`'s `success` variant. */
const CHIP_LOCKED_ON =
  'border-sage-200 bg-sage-50 text-sage-700 dark:border-sage-500/30 dark:bg-sage-500/10 dark:text-sage-300';

interface VoicePanelProviderChipsProps {
  t: (key: string) => string;
  sttProvider: string;
  ttsProvider: string;
  onSttProviderChange: (next: string) => void;
  onTtsProviderChange: (next: string) => void;
  voiceSettings: VoiceSettings | null;
  isInstallingPiper: boolean;
  piperInstall: VoiceInstallStatus | null;
  isSavingPendingKey: boolean;
  setPendingKeySlug: (slug: string | null) => void;
  setPendingKeyValue: (value: string) => void;
  handleRemoveProvider: (slug: string) => void | Promise<void>;
}

/**
 * Provider enable/disable chip row: managed cloud (locked on), Piper (local
 * TTS, no key required), and the external BYOK providers. Each chip's own
 * pill shape is intentional bespoke UI; the toggle inside it is the shared
 * `Switch` primitive so behaviour (role, aria-checked, keyboard) matches
 * every other switch in the app.
 */
const VoicePanelProviderChips = ({
  t,
  sttProvider,
  ttsProvider,
  onSttProviderChange,
  onTtsProviderChange,
  voiceSettings,
  isInstallingPiper,
  piperInstall,
  isSavingPendingKey,
  setPendingKeySlug,
  setPendingKeyValue,
  handleRemoveProvider,
}: VoicePanelProviderChipsProps) => {
  const piperEnabled = ttsProvider === 'piper';

  return (
    <SettingsSection title={t('voice.providers.title')} description={t('voice.providers.desc')}>
      <div className="px-4 py-3" data-testid="voice-providers-section">
        <div className="flex flex-wrap gap-2">
          {/* Cloud — always enabled, locked */}
          <div className={`${CHIP_SHELL} ${CHIP_LOCKED_ON}`}>
            <span>{t('voice.providers.chip.cloud')}</span>
            <SettingsSwitch
              id="voice-provider-chip-cloud"
              checked
              disabled
              onCheckedChange={() => {}}
              aria-label={t('voice.providers.chip.cloudAria')}
            />
          </div>

          {/* Piper — local TTS, no API key required. The chip opens the
              install/enable modal (which calls inference_install_piper and
              then voice_update_provider_settings on Enable). Toggling off
              routes TTS back to the managed cloud provider. */}
          <div className={`${CHIP_SHELL} ${CHIP_NEUTRAL}`}>
            <span>{t('voice.providers.chip.piper')}</span>
            <SettingsSwitch
              id="voice-provider-chip-piper"
              data-testid="voice-provider-chip-piper"
              checked={piperEnabled}
              // Stay disabled for the full install window: the local RPC
              // kickoff (`isInstallingPiper`) ends as soon as the start call
              // returns, but the install itself continues until the status
              // RPC reports `installed` / `error`. Combining both signals
              // prevents routing edits mid-install.
              disabled={isInstallingPiper || piperInstall?.state === 'installing'}
              onCheckedChange={next => {
                if (!next) {
                  onTtsProviderChange('cloud');
                } else {
                  setPendingKeySlug('piper');
                  setPendingKeyValue('');
                }
              }}
              aria-label={
                piperEnabled
                  ? `${t('voice.providers.chip.disableProvider')} ${t('voice.providers.chip.piper')}`
                  : `${t('voice.providers.chip.enableProvider')} ${t('voice.providers.chip.piper')}`
              }
            />
          </div>

          {/* External providers: Deepgram, ElevenLabs, OpenAI */}
          {Object.entries(BUILTIN_VOICE_PROVIDER_META).map(([slug, meta]) => {
            const enabled = (voiceSettings?.voiceProviders ?? []).some(p => p.slug === slug);
            return (
              <div
                key={slug}
                className={`${CHIP_SHELL} ${CHIP_NEUTRAL} ${meta.comingSoon ? 'opacity-60' : ''}`}>
                <span>
                  {meta.label}
                  {meta.comingSoon && (
                    <span className="ml-1 text-[10px] opacity-70">
                      ({t('voice.providers.chip.comingSoon')})
                    </span>
                  )}
                </span>
                <SettingsSwitch
                  id={`voice-provider-chip-${slug}`}
                  data-testid={`voice-provider-chip-${slug}`}
                  checked={enabled}
                  disabled={isSavingPendingKey || !!meta.comingSoon}
                  onCheckedChange={next => {
                    if (meta.comingSoon) return;
                    if (!next) {
                      void handleRemoveProvider(slug);
                      if (sttProvider === slug) onSttProviderChange('cloud');
                      if (ttsProvider === slug) onTtsProviderChange('cloud');
                    } else {
                      setPendingKeySlug(slug);
                      setPendingKeyValue('');
                    }
                  }}
                  aria-label={
                    enabled
                      ? `${t('voice.providers.chip.disableProvider')} ${meta.label}`
                      : `${t('voice.providers.chip.enableProvider')} ${meta.label}`
                  }
                />
              </div>
            );
          })}
        </div>
      </div>
    </SettingsSection>
  );
};

export default VoicePanelProviderChips;
