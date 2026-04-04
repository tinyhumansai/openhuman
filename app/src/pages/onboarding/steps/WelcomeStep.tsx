import OnboardingNextButton from '../components/OnboardingNextButton';

interface WelcomeStepProps {
  onNext: () => void;
}

const WelcomeStep = ({ onNext }: WelcomeStepProps) => {
  return (
    <div className="rounded-2xl border border-stone-200 bg-white p-8 shadow-soft animate-fade-up">
      <div className="text-center mb-6">
        <p className="text-xs font-semibold uppercase tracking-wider text-primary-600 mb-2">
          OpenHuman
        </p>
        <h1 className="text-2xl font-bold font-display mb-3 text-stone-900">Welcome</h1>
        <p className="text-stone-600 text-sm leading-relaxed">
          We&apos;ll walk you through a short setup:{' '}
          <span className="text-stone-700 font-medium">local AI</span>, permissions, tools, and
          skills. Nothing is permanent—you can adjust everything later in Settings.
        </p>
      </div>
      <OnboardingNextButton onClick={onNext} />
    </div>
  );
};

export default WelcomeStep;
