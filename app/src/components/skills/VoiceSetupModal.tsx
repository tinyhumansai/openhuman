/**
 * Voice Intelligence setup/enable modal.
 *
 * Two-step flow: if STT model isn't downloaded, directs to Local Model
 * settings. Otherwise, starts the voice server and shows success.
 */
import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import type { VoiceSkillStatus } from '../../features/voice/useVoiceSkillStatus';
import {
  openhumanVoiceServerStart,
  openhumanUpdateVoiceServerSettings,
} from '../../utils/tauriCommands/voice';

type Step = 'setup' | 'enable' | 'success';

interface Props {
  onClose: () => void;
  skillStatus: VoiceSkillStatus;
}

export default function VoiceSetupModal({ onClose, skillStatus }: Props) {
  const navigate = useNavigate();
  const { t } = useT();
  const { sttModelMissing, serverStatus } = skillStatus;

  const [step, setStep] = useState<Step>(sttModelMissing ? 'setup' : 'enable');
  const [isEnabling, setIsEnabling] = useState(false);
  const [enableError, setEnableError] = useState<string | null>(null);

  // Close on Escape key
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [onClose]);

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
    navigate('/settings/voice');
  };

  const handleGoToSettings = () => {
    onClose();
    navigate('/settings/voice');
  };

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
      onClick={e => {
        if (e.target === e.currentTarget) onClose();
      }}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="voice-setup-title"
        className="w-full max-w-md mx-4 rounded-2xl bg-white dark:bg-neutral-900 shadow-xl overflow-hidden animate-fade-up">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-stone-100 dark:border-neutral-800 px-5 py-4">
          <div className="flex items-center gap-3">
            <div className="w-9 h-9 rounded-xl bg-primary-50 flex items-center justify-center text-primary-600">
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.8}
                  d="M19 11a7 7 0 01-7 7m0 0a7 7 0 01-7-7m7 7v4m0 0H8m4 0h4m-4-8a3 3 0 01-3-3V5a3 3 0 116 0v6a3 3 0 01-3 3z"
                />
              </svg>
            </div>
            <div>
              <h2 id="voice-setup-title" className="text-sm font-semibold text-stone-900 dark:text-neutral-100">{t('skills.setup.voice.title')}</h2>
              <p className="text-xs text-stone-500 dark:text-neutral-400">
                {step === 'setup' && t('skills.setup.voice.stepSetup')}
                {step === 'enable' && t('skills.setup.voice.stepEnable')}
                {step === 'success' && t('skills.setup.voice.stepSuccess')}
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="w-7 h-7 rounded-lg flex items-center justify-center text-stone-400 dark:text-neutral-500 hover:text-stone-600 dark:hover:text-neutral-300 hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 transition-colors">
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Body */}
        <div className="px-5 py-4">
          {/* ─── Setup step: STT model missing ─── */}
          {step === 'setup' && (
            <div className="space-y-4">
              <div className="rounded-xl border border-amber-200 bg-amber-50 p-3 flex items-start gap-2">
                <svg className="w-4 h-4 text-amber-500 flex-shrink-0 mt-0.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-2.5L13.732 4c-.77-.833-1.964-.833-2.732 0L4.082 16.5c-.77.833.192 2.5 1.732 2.5z" />
                </svg>
                <div className="text-xs text-amber-700 leading-relaxed">
                  <p className="font-medium">{t('skills.setup.voice.sttNotReady')}</p>
                  <p className="mt-1">{t('skills.setup.voice.sttNotReadyDesc')}</p>
                </div>
              </div>

              <p className="text-xs text-stone-500 dark:text-neutral-400 leading-relaxed">
                {t('skills.setup.voice.sttReturnHint')}
              </p>

              <div className="flex flex-col gap-2">
                <button
                  type="button"
                  onClick={handleGoToLocalModel}
                  className="w-full rounded-xl bg-primary-500 px-4 py-2.5 text-sm font-medium text-white hover:bg-primary-600 transition-colors">
                  {t('skills.setup.voice.downloadSttBtn')}
                </button>
                <button
                  type="button"
                  onClick={onClose}
                  className="w-full rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-4 py-2.5 text-sm font-medium text-stone-600 dark:text-neutral-300 hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 transition-colors">
                  {t('common.cancel')}
                </button>
              </div>
            </div>
          )}

          {/* ─── Enable step ─── */}
          {step === 'enable' && (
            <div className="space-y-4">
              <div className="rounded-xl border border-sage-200 bg-sage-50 p-3 flex items-center gap-2">
                <svg className="w-4 h-4 text-sage-500 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                </svg>
                <span className="text-xs text-sage-700">{t('skills.setup.voice.sttReady')}</span>
              </div>

              <p className="text-xs text-stone-500 dark:text-neutral-400 leading-relaxed">
                {t('skills.setup.voice.enableDesc')}
              </p>

              <div className="space-y-2">
                <div className="flex items-center justify-between rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2.5">
                  <span className="text-sm text-stone-700 dark:text-neutral-200">{t('skills.setup.voice.hotkey')}</span>
                  <span className="text-xs font-mono text-stone-500 dark:text-neutral-400">{serverStatus?.hotkey ?? 'Fn'}</span>
                </div>
                <div className="flex items-center justify-between rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2.5">
                  <span className="text-sm text-stone-700 dark:text-neutral-200">{t('skills.setup.voice.activation')}</span>
                  <span className="text-xs text-stone-500 dark:text-neutral-400">{serverStatus?.activation_mode === 'push' ? t('voice.pushToTalk') : t('voice.tapToToggle')}</span>
                </div>
              </div>

              {enableError && (
                <div className="rounded-xl border border-coral-200 bg-coral-50 p-3 text-xs text-coral-700">
                  {enableError}
                </div>
              )}

              <button
                type="button"
                onClick={() => void handleEnable()}
                disabled={isEnabling}
                className="w-full rounded-xl bg-primary-500 px-4 py-2.5 text-sm font-medium text-white hover:bg-primary-600 disabled:opacity-50 transition-colors">
                {isEnabling ? t('skills.setup.voice.starting') : t('skills.setup.voice.startBtn')}
              </button>
            </div>
          )}

          {/* ─── Success step ─── */}
          {step === 'success' && (
            <div className="space-y-4 text-center py-2">
              <div className="mx-auto w-12 h-12 rounded-full bg-sage-50 flex items-center justify-center">
                <svg className="w-6 h-6 text-sage-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                </svg>
              </div>

              <div>
                <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">{t('skills.setup.voice.activeTitle')}</h3>
                <p className="text-center mt-1 text-xs text-stone-500 dark:text-neutral-400 leading-relaxed">
                  {t('skills.setup.voice.activeDescPrefix')} <span className="font-mono font-medium">{serverStatus?.hotkey ?? 'Fn'}</span> {t('skills.setup.voice.activeDescSuffix')}
                </p>
              </div>

              <div className="flex flex-col gap-2">
                <button
                  type="button"
                  onClick={handleGoToSettings}
                  className="w-full rounded-xl border border-primary-200 bg-primary-50 px-4 py-2.5 text-sm font-medium text-primary-700 hover:bg-primary-100 transition-colors">
                  {t('skills.setup.voice.customizeSettings')}
                </button>
                <button
                  type="button"
                  onClick={onClose}
                  className="w-full rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-4 py-2.5 text-sm font-medium text-stone-600 dark:text-neutral-300 hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 transition-colors">
                  {t('common.finish')}
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>,
    document.body
  );
}
