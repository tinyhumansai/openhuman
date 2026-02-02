import { useState } from 'react';
import { useNavigate } from 'react-router-dom';

import LottieAnimation from '../../components/LottieAnimation';
import ProgressIndicator from '../../components/ProgressIndicator';
import { userApi } from '../../services/api/userApi';
import { setOnboardedForUser } from '../../store/authSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import FeaturesStep from './steps/FeaturesStep';
import GetStartedStep from './steps/GetStartedStep';
import PrivacyStep from './steps/PrivacyStep';

const Onboarding = () => {
  const navigate = useNavigate();
  const dispatch = useAppDispatch();
  const user = useAppSelector(state => state.user.user);
  const [currentStep, setCurrentStep] = useState(0);
  const totalSteps = 3;

  // Lottie animation files for each step
  const stepAnimations = [
    '/lottie/wave.json', // Step 1 - Features
    '/lottie/safe3.json', // Step 2 - Privacy
    '/lottie/trophy.json', // Step 3 - Get Started
  ];

  const handleNext = () => {
    if (currentStep < totalSteps) {
      setCurrentStep(currentStep + 1);
    }
  };

  const handleComplete = async () => {
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
    navigate('/home');
  };

  const renderStep = () => {
    switch (currentStep) {
      case 1:
        return <FeaturesStep onNext={handleNext} />;
      case 2:
        return <PrivacyStep onNext={handleNext} />;
      case 3:
        return <GetStartedStep onComplete={handleComplete} />;
      default:
        return <FeaturesStep onNext={handleNext} />;
    }
  };

  return (
    <div className="min-h-screen relative flex items-center justify-center">
      <div className="relative z-10 max-w-lg w-full mx-4">
        <div className="flex justify-center mb-6">
          <LottieAnimation src={stepAnimations[currentStep - 1]} height={120} width={120} />
        </div>
        <ProgressIndicator currentStep={currentStep} totalSteps={totalSteps} />
        {renderStep()}
      </div>
    </div>
  );
};

export default Onboarding;
