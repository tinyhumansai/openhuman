import { useState } from 'react';
import { useNavigate } from 'react-router-dom';

import ProgressIndicator from '../../components/ProgressIndicator';
import { userApi } from '../../services/api/userApi';
import { setOnboardedForUser, setOnboardingTasksForUser } from '../../store/authSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import FeaturesStep from './steps/FeaturesStep';
import GetStartedStep from './steps/GetStartedStep';
import PrivacyStep from './steps/PrivacyStep';

interface OnboardingDraft {
  accessibilityPermissionGranted: boolean;
  localModelConsentGiven: boolean;
}

const Onboarding = () => {
  const navigate = useNavigate();
  const dispatch = useAppDispatch();
  const user = useAppSelector(state => state.user.user);
  const [currentStep, setCurrentStep] = useState(0);
  const [draft, setDraft] = useState<OnboardingDraft>({
    accessibilityPermissionGranted: false,
    localModelConsentGiven: false,
  });
  const totalSteps = 3;

  const handleNext = () => {
    if (currentStep < totalSteps - 1) {
      setCurrentStep(currentStep + 1);
    }
  };

  const handleAccessibilityNext = (accessibilityPermissionGranted: boolean) => {
    setDraft(prev => ({ ...prev, accessibilityPermissionGranted }));
    handleNext();
  };

  const handleLocalModelNext = (localModelConsentGiven: boolean) => {
    setDraft(prev => ({ ...prev, localModelConsentGiven }));
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
      case 1:
        return <PrivacyStep onNext={handleLocalModelNext} />;
      case 2:
        return <GetStartedStep onComplete={handleComplete} />;
      default:
        return <FeaturesStep onNext={handleAccessibilityNext} />;
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
