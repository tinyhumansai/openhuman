interface ProgressIndicatorProps {
  currentStep: number;
  totalSteps: number;
}

const ProgressIndicator = ({ currentStep, totalSteps }: ProgressIndicatorProps) => {
  return (
    <div className="flex items-center justify-center space-x-1.5 mb-6">
      {Array.from({ length: totalSteps }).map((_, index) => (
        <div
          key={index}
          className={`w-8 h-1 rounded-full ${
            index <= currentStep ? 'bg-sage-500' : 'bg-stone-300'
          }`}
        />
      ))}
    </div>
  );
};

export default ProgressIndicator;
