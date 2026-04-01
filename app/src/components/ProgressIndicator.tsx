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
          className={`w-8 h-0.5 rounded-full ${
            index <= currentStep ? 'bg-primary-500' : 'bg-stone-700'
          }`}
        />
      ))}
    </div>
  );
};

export default ProgressIndicator;
