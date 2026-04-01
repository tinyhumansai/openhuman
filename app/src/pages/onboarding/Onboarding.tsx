import { useState } from 'react';

import ProgressIndicator from '../../components/ProgressIndicator';
import { useUser } from '../../hooks/useUser';
import { userApi } from '../../services/api/userApi';
import { setOnboardedForUser, setOnboardingTasksForUser } from '../../store/authSlice';
import { useAppDispatch } from '../../store/hooks';
import { setOnboardingCompleted } from '../../utils/tauriCommands';
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

const Onboarding = ({ onComplete, onDefer }: OnboardingProps) => {
  const dispatch = useAppDispatch();
  const { user } = useUser();
  const [currentStep, setCurrentStep] = useState(0);
  const [draft, setDraft] = useState<OnboardingDraft>({
    localModelConsentGiven: false,
    localModelDownloadStarted: false,
    accessibilityPermissionGranted: false,
    enabledTools: [],
    connectedSources: [],
  });
  const totalSteps = 6;

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
        return <LocalAIStep onNext={handleLocalAINext} onBack={handleBack} />;
      case 2:
        return <ScreenPermissionsStep onNext={handleAccessibilityNext} onBack={handleBack} />;
      case 3:
        return <ToolsStep onNext={handleToolsNext} onBack={handleBack} />;
      case 4:
        return <SkillsStep onComplete={handleSkillsNext} onBack={handleBack} />;
      case 5:
        return <MnemonicStep onComplete={handleMnemonicComplete} onBack={handleBack} />;
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
    </div>
  );
};

export default Onboarding;
