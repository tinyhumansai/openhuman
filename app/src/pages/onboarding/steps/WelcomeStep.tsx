interface WelcomeStepProps {
  onNext: () => void;
}

const WelcomeStep = ({ onNext }: WelcomeStepProps) => {
  return (
    <div className="glass rounded-3xl p-8 shadow-large animate-fade-up">
      <div className="text-center mb-6">
        <p className="text-xs font-semibold uppercase tracking-wider text-primary-400 mb-2">
          OpenHuman
        </p>
        <h1 className="text-2xl font-bold font-display mb-3">Welcome</h1>
        <p className="opacity-70 text-sm leading-relaxed">
          We&apos;ll walk you through a short setup:{' '}
          <span className="text-stone-200 font-medium">local AI</span>, permissions, tools, and
          skills. Nothing is permanent—you can adjust everything later in Settings.
        </p>
      </div>
      <button
        type="button"
        onClick={onNext}
        className="btn-primary w-full py-2.5 text-sm font-medium rounded-xl">
        Continue
      </button>
    </div>
  );
};

export default WelcomeStep;
