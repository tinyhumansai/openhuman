import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

import ProgressIndicator from '../../components/ProgressIndicator';
import { useUser } from '../../hooks/useUser';
import { userApi } from '../../services/api/userApi';
import { setOnboardedForUser, setOnboardingTasksForUser } from '../../store/authSlice';
import { useAppDispatch } from '../../store/hooks';
import {
  openhumanLocalAiDownload,
  openhumanLocalAiDownloadAllAssets,
  setOnboardingCompleted,
} from '../../utils/tauriCommands';
import LocalAIStep from './steps/LocalAIStep';
import MnemonicStep from './steps/MnemonicStep';
import ScreenPermissionsStep from './steps/ScreenPermissionsStep';
import SkillsStep from './steps/SkillsStep';
import ToolsStep from './steps/ToolsStep';
import WelcomeStep from './steps/WelcomeStep';

interface OnboardingProps {
  onComplete?: () => void;
  onDefer?: () => void;
}

interface OnboardingDraft {
  localModelConsentGiven: boolean;
  localModelDownloadStarted: boolean;
  accessibilityPermissionGranted: boolean;
  enabledTools: string[];
  connectedSources: string[];
}

const LOCAL_AI_ERROR_DISMISS_MS = 10_000;

const Onboarding = ({ onComplete, onDefer }: OnboardingProps) => {
  const dispatch = useAppDispatch();
  const { user } = useUser();
  const [currentStep, setCurrentStep] = useState(0);
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const retryInFlightRef = useRef(false);
  const [draft, setDraft] = useState<OnboardingDraft>({
    localModelConsentGiven: false,
    localModelDownloadStarted: false,
    accessibilityPermissionGranted: false,
    enabledTools: [],
    connectedSources: [],
  });
  const totalSteps = 6;

  // Auto-dismiss the error banner after LOCAL_AI_ERROR_DISMISS_MS milliseconds.
  useEffect(() => {
    if (!downloadError) return;
    console.debug(
      '[Onboarding] Local AI download error surfaced; will auto-dismiss in',
      LOCAL_AI_ERROR_DISMISS_MS,
      'ms'
    );
    const timer = setTimeout(() => {
      setDownloadError(null);
    }, LOCAL_AI_ERROR_DISMISS_MS);
    return () => clearTimeout(timer);
  }, [downloadError]);

  // Re-fires both download commands when the user clicks "Retry" in the error banner.
  const retryDownload = useCallback(() => {
    if (retryInFlightRef.current) return;
    retryInFlightRef.current = true;
    console.debug('[Onboarding] User retrying Local AI download');
    setDownloadError(null);
    let errorReported = false;
    const reportError = (source: string, err: unknown) => {
      console.warn(`[Onboarding] Retry download failed (${source}):`, err);
      if (!errorReported) {
        errorReported = true;
        setDownloadError('Local AI setup encountered an issue');
      }
    };
    void Promise.allSettled([
      openhumanLocalAiDownload(false).catch((err: unknown) => reportError('ollama', err)),
      openhumanLocalAiDownloadAllAssets(false).catch((err: unknown) => reportError('assets', err)),
    ]).finally(() => {
      retryInFlightRef.current = false;
    });
  }, []);

  const handleNext = () => {
    if (currentStep < totalSteps - 1) {
      setCurrentStep(currentStep + 1);
    }
  };

  const handleBack = () => {
    if (currentStep > 0) {
      setCurrentStep(currentStep - 1);
    }
  };

  const handleLocalAINext = (result: { consentGiven: boolean; downloadStarted: boolean }) => {
    setDraft(prev => ({
      ...prev,
      localModelConsentGiven: result.consentGiven,
      localModelDownloadStarted: result.downloadStarted,
    }));
    handleNext();
  };

  const handleAccessibilityNext = (accessibilityPermissionGranted: boolean) => {
    setDraft(prev => ({ ...prev, accessibilityPermissionGranted }));
    handleNext();
  };

  const handleToolsNext = (enabledTools: string[]) => {
    setDraft(prev => ({ ...prev, enabledTools }));
    handleNext();
  };

  const handleSkillsNext = async (connectedSources: string[]) => {
    setDraft(prev => ({ ...prev, connectedSources }));

    // Persist onboarding tasks
    if (user?._id) {
      dispatch(
        setOnboardingTasksForUser({
          userId: user._id,
          tasks: {
            accessibilityPermissionGranted: draft.accessibilityPermissionGranted,
            localModelConsentGiven: draft.localModelConsentGiven,
            localModelDownloadStarted: draft.localModelDownloadStarted,
            enabledTools: draft.enabledTools,
            connectedSources,
          },
        })
      );
    }

    // Notify backend (best-effort — don't block onboarding completion)
    try {
      await userApi.onboardingComplete();
    } catch {
      console.warn('[onboarding] Failed to notify backend of onboarding completion');
    }

    // Advance to mnemonic step
    handleNext();
  };

  const handleMnemonicComplete = async () => {
    // Mark onboarded in Redux (belt-and-suspenders alongside config)
    if (user?._id) {
      dispatch(setOnboardedForUser({ userId: user._id, value: true }));
    }

    // Write onboarding_completed to core config (source of truth)
    try {
      await setOnboardingCompleted(true);
    } catch {
      console.warn('[onboarding] Failed to persist onboarding_completed to core config');
    }

    onComplete?.();
  };

  const renderStep = () => {
    switch (currentStep) {
      case 0:
        return <WelcomeStep onNext={handleNext} />;
      case 1:
        return (
          <LocalAIStep
            onNext={handleLocalAINext}
            onBack={handleBack}
            onDownloadError={setDownloadError}
          />
        );
      case 2:
        return <ScreenPermissionsStep onNext={handleAccessibilityNext} onBack={handleBack} />;
      case 3:
        return <ToolsStep onNext={handleToolsNext} onBack={handleBack} />;
      case 4:
        return <SkillsStep onNext={handleSkillsNext} onBack={handleBack} />;
      case 5:
        return <MnemonicStep onNext={handleMnemonicComplete} onBack={handleBack} />;
      default:
        return null;
    }
  };

  return (
    <div className="min-h-full relative flex items-center justify-center">
      {onDefer && (
        <div className="fixed top-4 right-0 z-20 sm:top-6 sm:right-6">
          <button
            type="button"
            onClick={onDefer}
            className="text-sm text-white hover:text-stone-200 transition-colors">
            Skip
          </button>
        </div>
      )}
      <div className="relative z-10 max-w-lg w-full mx-4">
        <ProgressIndicator currentStep={currentStep} totalSteps={totalSteps} />
        {renderStep()}
      </div>
      {downloadError &&
        createPortal(
          <div
            role="alert"
            aria-live="assertive"
            className="fixed bottom-4 left-4 z-[9997] w-[320px] animate-fade-up">
            <div className="bg-stone-900 border border-coral-500/30 rounded-2xl shadow-large p-4">
              <div className="flex items-start gap-3">
                {/* Warning icon */}
                <svg
                  aria-hidden="true"
                  className="flex-shrink-0 mt-0.5 w-5 h-5 text-coral-400"
                  viewBox="0 0 20 20"
                  fill="currentColor">
                  <path
                    fillRule="evenodd"
                    d="M8.485 2.495c.673-1.167 2.357-1.167 3.03 0l6.28 10.875c.673 1.167-.17 2.625-1.516 2.625H3.72c-1.347 0-2.189-1.458-1.515-2.625L8.485 2.495zM10 5a.75.75 0 01.75.75v3.5a.75.75 0 01-1.5 0v-3.5A.75.75 0 0110 5zm0 9a1 1 0 100-2 1 1 0 000 2z"
                    clipRule="evenodd"
                  />
                </svg>
                <div className="flex-1 min-w-0">
                  <p className="text-sm font-medium text-stone-100">{downloadError}</p>
                  <p className="mt-0.5 text-xs text-stone-400">
                    You can retry or continue — downloads can be resumed later.
                  </p>
                  <button
                    type="button"
                    onClick={retryDownload}
                    className="mt-2 text-xs font-medium text-primary-400 hover:text-primary-300 transition-colors">
                    Retry
                  </button>
                </div>
                {/* Dismiss button */}
                <button
                  type="button"
                  aria-label="Dismiss Local AI error"
                  onClick={() => setDownloadError(null)}
                  className="flex-shrink-0 text-stone-500 hover:text-stone-300 transition-colors">
                  <svg
                    aria-hidden="true"
                    className="w-4 h-4"
                    viewBox="0 0 20 20"
                    fill="currentColor">
                    <path d="M6.28 5.22a.75.75 0 00-1.06 1.06L8.94 10l-3.72 3.72a.75.75 0 101.06 1.06L10 11.06l3.72 3.72a.75.75 0 101.06-1.06L11.06 10l3.72-3.72a.75.75 0 00-1.06-1.06L10 8.94 6.28 5.22z" />
                  </svg>
                </button>
              </div>
            </div>
          </div>,
          document.body
        )}
    </div>
  );
};

export default Onboarding;
