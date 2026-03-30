import { useState } from 'react';
import { useNavigate } from 'react-router-dom';

import ProgressIndicator from '../../components/ProgressIndicator';
import { userApi } from '../../services/api/userApi';
import { setOnboardedForUser, setOnboardingTasksForUser } from '../../store/authSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import LocalAIStep from './steps/LocalAIStep';
import ScreenPermissionsStep from './steps/ScreenPermissionsStep';
import WelcomeStep from './steps/WelcomeStep';
import SkillsStep from './steps/SkillsStep';
import ToolsStep from './steps/ToolsStep';

interface OnboardingDraft {
  localModelConsentGiven: boolean;
  localModelDownloadStarted: boolean;
  accessibilityPermissionGranted: boolean;
  enabledTools: string[];
}

const Onboarding = () => {
  const navigate = useNavigate();
  const dispatch = useAppDispatch();
  const user = useAppSelector(state => state.user.user);
  const [currentStep, setCurrentStep] = useState(0);
  const [draft, setDraft] = useState<OnboardingDraft>({
    localModelConsentGiven: false,
    localModelDownloadStarted: false,
    accessibilityPermissionGranted: false,
    enabledTools: [],
  });
  const totalSteps = 5;

  const handleNext = () => {
    if (currentStep < totalSteps - 1) {
      setCurrentStep(currentStep + 1);
    }
  };

  const handleLocalAINext = (result: {
    consentGiven: boolean;
    downloadStarted: boolean;
  }) => {
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

  const handleComplete = async (connectedSources: string[]) => {
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
    if (user?._id) {
      dispatch(setOnboardedForUser({ userId: user._id, value: true }));
    }
    navigate('/mnemonic');
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
        return <SkillsStep onComplete={handleComplete} />;
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
