import { useState } from 'react';

import ProgressIndicator from '../../components/ProgressIndicator';
import { userApi } from '../../services/api/userApi';
import { setOnboardedForUser, setOnboardingTasksForUser } from '../../store/authSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import { openhumanWorkspaceOnboardingFlagSet } from '../../utils/tauriCommands';
import LocalAIStep from './steps/LocalAIStep';
import MnemonicStep from './steps/MnemonicStep';
import ScreenPermissionsStep from './steps/ScreenPermissionsStep';
import SkillsStep from './steps/SkillsStep';
import ToolsStep from './steps/ToolsStep';
import WelcomeStep from './steps/WelcomeStep';

interface OnboardingProps {
  onComplete?: () => void;
}

interface OnboardingDraft {
  localModelConsentGiven: boolean;
  localModelDownloadStarted: boolean;
  accessibilityPermissionGranted: boolean;
  enabledTools: string[];
  connectedSources: string[];
}

const Onboarding = ({ onComplete }: OnboardingProps) => {
  const dispatch = useAppDispatch();
  const user = useAppSelector(state => state.user.user);
  const [currentStep, setCurrentStep] = useState(0);
  const [draft, setDraft] = useState<OnboardingDraft>({
    localModelConsentGiven: false,
    localModelDownloadStarted: false,
    accessibilityPermissionGranted: false,
    enabledTools: [],
    connectedSources: [],
  });
  const totalSteps = 7;

  const handleNext = () => {
    if (currentStep < totalSteps - 1) {
      setCurrentStep(currentStep + 1);
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

    // Notify backend
    try {
      await userApi.onboardingComplete();
    } catch (e) {
      const msg =
        e &&
        typeof e === 'object' &&
        'error' in e &&
        typeof (e as { error: unknown }).error === 'string'
          ? (e as { error: string }).error
          : 'Failed to complete onboarding. Please try again.';
      throw new Error(msg);
    }

    // Advance to mnemonic step
    handleNext();
  };

  const handleMnemonicComplete = async () => {
    // Mark onboarded in Redux
    if (user?._id) {
      dispatch(setOnboardedForUser({ userId: user._id, value: true }));
    }

    // Write workspace flag so the overlay won't show again
    try {
      await openhumanWorkspaceOnboardingFlagSet(true);
    } catch {
      // Non-critical — Redux state is the primary gate
    }

    onComplete?.();
  };

  const renderStep = () => {
    switch (currentStep) {
      case 0:
        return <WelcomeStep onNext={handleNext} />;
      case 1:
        return <LocalAIStep onNext={handleLocalAINext} />;
      case 2:
        return <ScreenPermissionsStep onNext={handleAccessibilityNext} />;
      case 3:
        return <ToolsStep onNext={handleToolsNext} />;
      case 4:
        return <SkillsStep onComplete={handleSkillsNext} />;
      case 5:
        return <MnemonicStep onComplete={handleMnemonicComplete} />;
      default:
        return null;
    }
  };

  return (
    <div className="min-h-full relative flex items-center justify-center">
      <div className="relative z-10 max-w-lg w-full mx-4">
        <ProgressIndicator currentStep={currentStep} totalSteps={totalSteps} />
        {renderStep()}
      </div>
    </div>
  );
};

export default Onboarding;
