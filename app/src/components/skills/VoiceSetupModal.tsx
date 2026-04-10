/**
 * Voice Intelligence setup/enable modal.
 *
 * Two-step flow: if STT model isn't downloaded, directs to Local Model
 * settings. Otherwise, starts the voice server and shows success.
 */
import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { useNavigate } from 'react-router-dom';

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
      setEnableError(error instanceof Error ? error.message : 'Failed to start voice server');
    } finally {
      setIsEnabling(false);
    }
  };

  const handleGoToLocalModel = () => {
    onClose();
    navigate('/settings/local-model');
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
        className="w-full max-w-md mx-4 rounded-2xl bg-white shadow-xl overflow-hidden animate-fade-up">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-stone-100 px-5 py-4">
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
              <h2 id="voice-setup-title" className="text-sm font-semibold text-stone-900">Voice Intelligence</h2>
              <p className="text-xs text-stone-500">
                {step === 'setup' && 'Model download required'}
                {step === 'enable' && 'Start voice server'}
                {step === 'success' && 'Ready to go'}
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="w-7 h-7 rounded-lg flex items-center justify-center text-stone-400 hover:text-stone-600 hover:bg-stone-100 transition-colors">
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
                  <p className="font-medium">Speech-to-text model not ready</p>
                  <p className="mt-1">Voice Intelligence requires a local Whisper model for transcription. Download it from the Local Model settings.</p>
                </div>
              </div>

              <p className="text-xs text-stone-500 leading-relaxed">
                Once the STT model is downloaded, you can return here to enable voice dictation and voice-driven AI chat.
              </p>

              <div className="flex flex-col gap-2">
                <button
                  type="button"
                  onClick={handleGoToLocalModel}
                  className="w-full rounded-xl bg-primary-500 px-4 py-2.5 text-sm font-medium text-white hover:bg-primary-600 transition-colors">
                  Download STT Model
                </button>
                <button
                  type="button"
                  onClick={onClose}
                  className="w-full rounded-xl border border-stone-200 bg-stone-50 px-4 py-2.5 text-sm font-medium text-stone-600 hover:bg-stone-100 transition-colors">
                  Cancel
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
                <span className="text-xs text-sage-700">Speech-to-text model ready</span>
              </div>

              <p className="text-xs text-stone-500 leading-relaxed">
                Start the voice server to use dictation and voice-driven chat. Press the hotkey to toggle recording.
              </p>

              <div className="space-y-2">
                <div className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2.5">
                  <span className="text-sm text-stone-700">Hotkey</span>
                  <span className="text-xs font-mono text-stone-500">{serverStatus?.hotkey ?? 'Fn'}</span>
                </div>
                <div className="flex items-center justify-between rounded-xl border border-stone-200 bg-stone-50 px-3 py-2.5">
                  <span className="text-sm text-stone-700">Activation</span>
                  <span className="text-xs text-stone-500">{serverStatus?.activation_mode === 'push' ? 'Push-to-talk' : 'Tap to toggle'}</span>
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
                {isEnabling ? 'Starting...' : 'Start Voice Server'}
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
                <h3 className="text-sm font-semibold text-stone-900">Voice Intelligence is Active</h3>
                <p className="mt-1 text-xs text-stone-500 leading-relaxed">
                  Press <span className="font-mono font-medium">{serverStatus?.hotkey ?? 'Fn'}</span> to start dictating. Your voice will be transcribed and sent to your AI assistant.
                </p>
              </div>

              <div className="flex flex-col gap-2">
                <button
                  type="button"
                  onClick={handleGoToSettings}
                  className="w-full rounded-xl border border-primary-200 bg-primary-50 px-4 py-2.5 text-sm font-medium text-primary-700 hover:bg-primary-100 transition-colors">
                  Customize Settings
                </button>
                <button
                  type="button"
                  onClick={onClose}
                  className="w-full rounded-xl border border-stone-200 bg-stone-50 px-4 py-2.5 text-sm font-medium text-stone-600 hover:bg-stone-100 transition-colors">
                  Done
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
