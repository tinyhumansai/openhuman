/**
 * Voice Intelligence setup/enable modal.
 *
 * Two-step flow: if STT model isn't downloaded, directs to Local Model
 * settings. Otherwise, starts the voice server and shows success.
 */
import { useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import type { VoiceSkillStatus } from '../../features/voice/useVoiceSkillStatus';
import { useT } from '../../lib/i18n/I18nContext';
import {
  openhumanUpdateVoiceServerSettings,
  openhumanVoiceServerStart,
} from '../../utils/tauriCommands/voice';
import { settingsNavState } from '../settings/modal/settingsOverlay';
import { CheckIcon, WarningIcon } from '../ui';
import Button from '../ui/Button';
import {
  SetupNotice,
  SetupSettingRow,
  SetupSuccess,
  SkillSetupModalShell,
} from './SkillSetupPrimitives';

type Step = 'setup' | 'enable' | 'success';

interface Props {
  onClose: () => void;
  skillStatus: VoiceSkillStatus;
}

export default function VoiceSetupModal({ onClose, skillStatus }: Props) {
  const navigate = useNavigate();
  const location = useLocation();
  const { t } = useT();
  const { sttModelMissing, serverStatus } = skillStatus;

  const [step, setStep] = useState<Step>(sttModelMissing ? 'setup' : 'enable');
  const [isEnabling, setIsEnabling] = useState(false);
  const [enableError, setEnableError] = useState<string | null>(null);

  const handleEnable = async () => {
    setIsEnabling(true);
    setEnableError(null);
    try {
      // Enable auto-start in settings
      await openhumanUpdateVoiceServerSettings({ auto_start: true });
      // Start the voice server
      await openhumanVoiceServerStart();
      setStep('success');
    } catch (error) {
      setEnableError(error instanceof Error ? error.message : t('skills.setup.voice.startError'));
    } finally {
      setIsEnabling(false);
    }
  };

  const handleGoToLocalModel = () => {
    onClose();
    // STT model install lives on the Voice settings panel (PR 2). The
    // legacy `/settings/local-model` route handled Ollama assets only.
    navigate('/settings/voice', settingsNavState(location));
  };

  const handleGoToSettings = () => {
    onClose();
    navigate('/settings/voice', settingsNavState(location));
  };

  return (
    <SkillSetupModalShell
      onClose={onClose}
      title={t('skills.setup.voice.title')}
      titleId="voice-setup-title"
      subtitle={
        <>
          {step === 'setup' && t('skills.setup.voice.stepSetup')}
          {step === 'enable' && t('skills.setup.voice.stepEnable')}
          {step === 'success' && t('skills.setup.voice.stepSuccess')}
        </>
      }
      icon={
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.8}
            d="M19 11a7 7 0 01-7 7m0 0a7 7 0 01-7-7m7 7v4m0 0H8m4 0h4m-4-8a3 3 0 01-3-3V5a3 3 0 116 0v6a3 3 0 01-3 3z"
          />
        </svg>
      }>
      {/* ─── Setup step: STT model missing ─── */}
      {step === 'setup' && (
        <div className="space-y-4">
          <SetupNotice tone="amber" icon={<WarningIcon className="w-4 h-4 text-amber-500" />}>
            <div className="text-xs text-amber-700 leading-relaxed">
              <p className="font-medium">{t('skills.setup.voice.sttNotReady')}</p>
              <p className="mt-1">{t('skills.setup.voice.sttNotReadyDesc')}</p>
            </div>
          </SetupNotice>

          <p className="text-xs text-content-muted leading-relaxed">
            {t('skills.setup.voice.sttReturnHint')}
          </p>

          <div className="flex flex-col gap-2">
            <Button variant="primary" size="lg" onClick={handleGoToLocalModel} className="w-full">
              {t('skills.setup.voice.downloadSttBtn')}
            </Button>
            <Button variant="secondary" size="lg" onClick={onClose} className="w-full">
              {t('common.cancel')}
            </Button>
          </div>
        </div>
      )}

      {/* ─── Enable step ─── */}
      {step === 'enable' && (
        <div className="space-y-4">
          <SetupNotice tone="sage" icon={<CheckIcon className="w-4 h-4 text-sage-500" />}>
            {t('skills.setup.voice.sttReady')}
          </SetupNotice>

          <p className="text-xs text-content-muted leading-relaxed">
            {t('skills.setup.voice.enableDesc')}
          </p>

          <div className="space-y-2">
            <SetupSettingRow
              label={t('skills.setup.voice.hotkey')}
              value={serverStatus?.hotkey ?? 'Fn'}
              mono
            />
            <SetupSettingRow
              label={t('skills.setup.voice.activation')}
              value={
                serverStatus?.activation_mode === 'push'
                  ? t('voice.pushToTalk')
                  : t('voice.tapToToggle')
              }
            />
          </div>

          {enableError && <SetupNotice tone="coral">{enableError}</SetupNotice>}

          <Button
            variant="primary"
            size="lg"
            onClick={() => void handleEnable()}
            disabled={isEnabling}
            className="w-full">
            {isEnabling ? t('skills.setup.voice.starting') : t('skills.setup.voice.startBtn')}
          </Button>
        </div>
      )}

      {/* ─── Success step ─── */}
      {step === 'success' && (
        <SetupSuccess
          title={t('skills.setup.voice.activeTitle')}
          description={
            <>
              {t('skills.setup.voice.activeDescPrefix')}{' '}
              <span className="font-mono font-medium">{serverStatus?.hotkey ?? 'Fn'}</span>{' '}
              {t('skills.setup.voice.activeDescSuffix')}
            </>
          }
          settingsLabel={t('skills.setup.voice.customizeSettings')}
          finishLabel={t('common.finish')}
          onSettings={handleGoToSettings}
          onFinish={onClose}
        />
      )}
    </SkillSetupModalShell>
  );
}
