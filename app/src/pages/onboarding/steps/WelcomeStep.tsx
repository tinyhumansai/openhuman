import WhatLeavesLink from '../../../features/privacy/WhatLeavesLink';
import OnboardingNextButton from '../components/OnboardingNextButton';

interface WelcomeStepProps {
  onNext: () => void;
}

const WelcomeStep = ({ onNext }: WelcomeStepProps) => {
  return (
    <div
      data-testid="onboarding-welcome-step"
      className="rounded-2xl bg-white p-10 shadow-soft animate-fade-up">
      <div className="flex flex-col items-center text-center">
        <img src="/logo.png" alt="OpenHuman" className="w-20 h-20 rounded-2xl mb-5" />
        <h1 className="text-3xl font-display text-stone-900 mb-3 leading-tight">
          Hi. I'm OpenHuman.
        </h1>
        <p className="text-stone-500 text-sm leading-relaxed max-w-sm">
          Your super-intelligent AI assistant that runs on your computer. Private, simple, and
          extremely powerful.
        </p>
      </div>
      <div className="mt-8">
        <OnboardingNextButton label="Let's Start" onClick={onNext} />
      </div>
      <div className="mt-4 flex justify-center">
        <WhatLeavesLink />
      </div>
    </div>
  );
};

export default WelcomeStep;
