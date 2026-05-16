interface ProgressIndicatorProps {
  currentStep: number;
  totalSteps: number;
}

const ProgressIndicator = ({ currentStep, totalSteps }: ProgressIndicatorProps) => {
  return (
    <div
      role="tablist"
      aria-label="Progress steps"
      className="flex items-center justify-center space-x-2"
    >
      {Array.from({ length: totalSteps }).map((_, index) => {
        const isCurrent = index === currentStep;
        return (
          <div
            key={index}
            role="tab"
            aria-label={`Step ${index + 1} of ${totalSteps}`}
            aria-current={isCurrent ? 'step' : undefined}
            aria-selected={isCurrent}
            className={`w-2 h-2 rounded-full transition-colors ${
              isCurrent ? 'bg-stone-800' : 'bg-stone-300'
            }`}
          />
        );
      })}
    </div>
  );
};

export default ProgressIndicator;
